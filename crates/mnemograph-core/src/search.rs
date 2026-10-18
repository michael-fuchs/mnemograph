//! Full-text search: a Tantivy index derived from the SQLite data (ADR 0003).
//!
//! Every text is indexed twice, once stemmed as German and once as English, and queries search
//! both. The index is never the source of truth: it can always be rebuilt from SQLite.

use std::path::Path;
use std::sync::Mutex;

use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{
    Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer,
};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

/// Changes whenever the index layout changes, so that older indexes are rebuilt.
const LAYOUT: &str = "mnemograph-search-1";
const WRITER_MEMORY: usize = 20_000_000;
const SNIPPET_CHARS: usize = 200;

/// Something that matched a search, before it is resolved against the database.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawHit {
    pub(crate) item: Item,
    pub(crate) snippet: String,
    pub(crate) score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Item {
    Page(i64),
    Note(i64),
}

impl Item {
    fn key(self) -> String {
        match self {
            Item::Page(id) => format!("page:{id}"),
            Item::Note(id) => format!("note:{id}"),
        }
    }

    fn parse(key: &str) -> Option<Self> {
        let (kind, id) = key.split_once(':')?;
        let id = id.parse().ok()?;
        match kind {
            "page" => Some(Item::Page(id)),
            "note" => Some(Item::Note(id)),
            _ => None,
        }
    }
}

/// The text of a page or note as the index sees it.
pub(crate) struct IndexedText<'a> {
    pub(crate) item: Item,
    /// Title and aliases of a page; empty for a note.
    pub(crate) names: &'a [&'a str],
    pub(crate) page_type: Option<&'a str>,
    pub(crate) body: &'a str,
}

struct Fields {
    key: Field,
    page_type: Field,
    names_de: Field,
    names_en: Field,
    body_de: Field,
    body_en: Field,
}

pub(crate) struct SearchIndex {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: Fields,
}

impl SearchIndex {
    /// Opens the index in `dir`, creating it (or recreating an unreadable one) if necessary.
    pub(crate) fn open(dir: &Path) -> tantivy::Result<Self> {
        let (schema, fields) = schema();
        std::fs::create_dir_all(dir)?;
        let index = match Index::open_in_dir(dir) {
            Ok(index) if index.schema() == schema => index,
            _ => {
                // Derived data: an unreadable or outdated index is simply recreated.
                std::fs::remove_dir_all(dir)?;
                std::fs::create_dir_all(dir)?;
                Index::create_in_dir(dir, schema)?
            }
        };
        Self::from_index(index, fields)
    }

    pub(crate) fn in_memory() -> tantivy::Result<Self> {
        let (schema, fields) = schema();
        Self::from_index(Index::create_in_ram(schema), fields)
    }

    fn from_index(index: Index, fields: Fields) -> tantivy::Result<Self> {
        for language in [Language::German, Language::English] {
            index
                .tokenizers()
                .register(analyzer_name(language), analyzer(language));
        }
        let writer = index.writer_with_num_threads(1, WRITER_MEMORY)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            fields,
        })
    }

    /// The change counter the index reflects, if it was written by this layout.
    pub(crate) fn change_counter(&self) -> Option<i64> {
        let payload = self.index.load_metas().ok()?.payload?;
        let (layout, counter) = payload.split_once(' ')?;
        (layout == LAYOUT).then(|| counter.parse().ok())?
    }

    /// Replaces the indexed texts of the given items and commits, recording `change_counter`.
    /// Items with `None` text are removed.
    pub(crate) fn update(
        &self,
        texts: &[(Item, Option<IndexedText<'_>>)],
        change_counter: i64,
    ) -> tantivy::Result<()> {
        let mut writer = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        for (item, text) in texts {
            writer.delete_term(Term::from_field_text(self.fields.key, &item.key()));
            if let Some(text) = text {
                writer.add_document(self.document(text))?;
            }
        }
        self.commit(&mut writer, change_counter)
    }

    /// Replaces the whole index with `texts`.
    pub(crate) fn rebuild<'a>(
        &self,
        texts: impl IntoIterator<Item = IndexedText<'a>>,
        change_counter: i64,
    ) -> tantivy::Result<()> {
        let mut writer = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        writer.delete_all_documents()?;
        for text in texts {
            writer.add_document(self.document(&text))?;
        }
        self.commit(&mut writer, change_counter)
    }

    fn commit(&self, writer: &mut IndexWriter, change_counter: i64) -> tantivy::Result<()> {
        let mut commit = writer.prepare_commit()?;
        commit.set_payload(&format!("{LAYOUT} {change_counter}"));
        commit.commit()?;
        self.reader.reload()
    }

    fn document(&self, text: &IndexedText<'_>) -> TantivyDocument {
        let f = &self.fields;
        let names = text.names.join("\n");
        let mut document = doc!(
            f.key => text.item.key(),
            f.names_de => names.clone(),
            f.names_en => names,
            f.body_de => text.body,
            f.body_en => text.body,
        );
        if let Some(page_type) = text.page_type {
            document.add_text(f.page_type, page_type);
        }
        document
    }

    /// Searches pages and notes. With `page_type`, only pages of that type match.
    pub(crate) fn search(
        &self,
        query: &str,
        page_type: Option<&str>,
        limit: usize,
    ) -> tantivy::Result<Vec<RawHit>> {
        let f = &self.fields;
        let mut parser = QueryParser::for_index(
            &self.index,
            vec![f.names_de, f.names_en, f.body_de, f.body_en],
        );
        parser.set_field_boost(f.names_de, 3.0);
        parser.set_field_boost(f.names_en, 3.0);
        // Agents write free text; syntax errors just lose the broken part of the query.
        let (text_query, _errors) = parser.parse_query_lenient(query);
        let query: Box<dyn Query> = match page_type {
            None => text_query,
            Some(page_type) => Box::new(BooleanQuery::new(vec![
                (Occur::Must, text_query),
                (
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(f.page_type, page_type),
                        IndexRecordOption::Basic,
                    )),
                ),
            ])),
        };

        let searcher = self.reader.searcher();
        let mut snippets = SnippetGenerator::create(&searcher, &*query, f.body_de)?;
        snippets.set_max_num_chars(SNIPPET_CHARS);
        let mut hits = Vec::new();
        for (score, address) in
            searcher.search(&*query, &TopDocs::with_limit(limit).order_by_score())?
        {
            let document: TantivyDocument = searcher.doc(address)?;
            let Some(item) = document
                .get_first(f.key)
                .and_then(|v| v.as_str())
                .and_then(Item::parse)
            else {
                continue;
            };
            let body = document
                .get_first(f.body_de)
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            hits.push(RawHit {
                item,
                snippet: snippet_text(&snippets.snippet(body), body),
                score,
            });
        }
        Ok(hits)
    }
}

/// Renders a snippet as plain text with matches in **bold**. A short body is shown whole; a
/// fragment of a longer one is marked with … where it was cut. Falls back to the start of the
/// body when the match was only in the title or aliases.
fn snippet_text(snippet: &tantivy::snippet::Snippet, body: &str) -> String {
    let short = body.chars().count() <= SNIPPET_CHARS;
    if snippet.fragment().is_empty() {
        let start: String = body.chars().take(SNIPPET_CHARS).collect();
        return if short { start } else { format!("{start}…") };
    }
    let fragment = snippet.fragment_range();
    let (from, to) = if short {
        (0, body.len())
    } else {
        (fragment.start, fragment.end)
    };
    let mut text = String::new();
    if from > 0 {
        text.push('…');
    }
    let mut position = from;
    for range in snippet.highlighted() {
        let (start, end) = (fragment.start + range.start, fragment.start + range.end);
        text.push_str(&body[position..start]);
        text.push_str("**");
        text.push_str(&body[start..end]);
        text.push_str("**");
        position = end;
    }
    text.push_str(&body[position..to]);
    if to < body.len() {
        text.push('…');
    }
    text
}

fn schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let text = |language, stored: bool| {
        let options = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(analyzer_name(language))
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );
        if stored { options | STORED } else { options }
    };
    let fields = Fields {
        key: builder.add_text_field("key", STRING | STORED),
        page_type: builder.add_text_field("page_type", STRING),
        names_de: builder.add_text_field("names_de", text(Language::German, false)),
        names_en: builder.add_text_field("names_en", text(Language::English, false)),
        body_de: builder.add_text_field("body_de", text(Language::German, true)),
        body_en: builder.add_text_field("body_en", text(Language::English, false)),
    };
    (builder.build(), fields)
}

fn analyzer_name(language: Language) -> &'static str {
    match language {
        Language::German => "mnemograph_de",
        _ => "mnemograph_en",
    }
}

fn analyzer(language: Language) -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::new(language))
        .build()
}
