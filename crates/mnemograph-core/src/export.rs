//! Export of all knowledge into plain files, independent of the storage engine.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::json;

use crate::{Error, Knowledge, Page, Result};

/// The longest file name stem (in characters) an exported page gets.
const MAX_STEM_CHARS: usize = 100;

/// What an export wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSummary {
    pub pages: usize,
    pub notes: usize,
}

impl Knowledge {
    /// Writes every page as `<dir>/pages/<title>.md` (Markdown with a YAML header) and all notes
    /// to `<dir>/notes.jsonl`. `dir` must not exist yet or be empty.
    pub fn export(&self, dir: &Path) -> Result<ExportSummary> {
        if dir.exists() && fs::read_dir(dir)?.next().is_some() {
            return Err(Error::ExportTargetNotEmpty(dir.display().to_string()));
        }
        let pages_dir = dir.join("pages");
        fs::create_dir_all(&pages_dir)?;

        // Oldest first, so a page keeps its file name when newer pages with similar titles come.
        let pages = self.pages_in_creation_order()?;
        let mut taken = HashSet::new();
        for page in &pages {
            let name = unique_file_name(&page.title, &mut taken);
            fs::write(pages_dir.join(name), page_file(page))?;
        }

        let notes = self.notes()?;
        let mut file = fs::File::create(dir.join("notes.jsonl"))?;
        for note in &notes {
            let links: Vec<&str> = note.links.iter().map(|l| l.title.as_str()).collect();
            let line = json!({
                "id": note.id,
                "created_at": note.created_at,
                "author": note.author.as_ref().map(ToString::to_string),
                "body": note.body,
                "links": links,
            });
            writeln!(file, "{line}")?;
        }
        file.sync_all()?;

        Ok(ExportSummary {
            pages: pages.len(),
            notes: notes.len(),
        })
    }
}

/// Markdown with a YAML header. Strings are written as JSON strings, which YAML reads as
/// double-quoted scalars.
fn page_file(page: &Page) -> String {
    let mut text = String::from("---\n");
    text += &format!("title: {}\n", json!(page.title));
    if let Some(page_type) = &page.page_type {
        text += &format!("type: {}\n", json!(page_type));
    }
    if !page.aliases.is_empty() {
        text += &format!("aliases: {}\n", json!(page.aliases));
    }
    text += &format!("revision: {}\n", page.revision);
    text += &format!("created_at: {}\n", json!(page.created_at));
    text += &format!("updated_at: {}\n", json!(page.updated_at));
    if let Some(author) = &page.author {
        text += &format!("updated_by: {}\n", json!(author.to_string()));
    }
    text += "---\n\n";
    text += &page.body;
    if !page.body.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// A file name for the title that is safe on common file systems: letters and digits are
/// kept, everything else becomes a single `-`. Names that would collide (also when compared
/// case-insensitively) get a numeric suffix.
fn unique_file_name(title: &str, taken: &mut HashSet<String>) -> String {
    let stem = file_stem(title);
    let mut candidate = stem.clone();
    let mut counter = 2;
    while !taken.insert(candidate.to_lowercase()) {
        candidate = format!("{stem}-{counter}");
        counter += 1;
    }
    format!("{candidate}.md")
}

fn file_stem(title: &str) -> String {
    let mut stem = String::new();
    for c in title.chars().take(MAX_STEM_CHARS) {
        if c.is_alphanumeric() {
            stem.push(c);
        } else if !stem.is_empty() && !stem.ends_with('-') {
            stem.push('-');
        }
    }
    let stem = stem.trim_end_matches('-');
    if stem.is_empty() {
        "page".to_owned()
    } else {
        stem.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stems_are_safe() {
        assert_eq!(file_stem("Rauthy (IdP)"), "Rauthy-IdP");
        assert_eq!(
            file_stem("C++ / Rust: a comparison?"),
            "C-Rust-a-comparison"
        );
        assert_eq!(file_stem("Größe über alles"), "Größe-über-alles");
        assert_eq!(file_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(file_stem("***"), "page");
        assert_eq!(file_stem(&"x".repeat(300)).len(), MAX_STEM_CHARS);
    }

    #[test]
    fn colliding_names_get_suffixes() {
        let mut taken = HashSet::new();
        assert_eq!(unique_file_name("C++", &mut taken), "C.md");
        assert_eq!(unique_file_name("C#", &mut taken), "C-2.md");
        assert_eq!(unique_file_name("c", &mut taken), "c-3.md");
    }
}
