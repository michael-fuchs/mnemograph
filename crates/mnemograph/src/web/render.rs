//! Renders page and note bodies (mnemograph-flavored Markdown) to HTML for the web UI.
//!
//! Bodies are written by agents, so rendering must be safe: raw HTML is not rendered and
//! dangerous link schemes are dropped (comrak's defaults, which stay off).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write};

use comrak::adapters::CodefenceRendererAdapter;
use comrak::nodes::{NodeLink, NodeValue, Sourcepos};
use comrak::options::Plugins;
use comrak::{Arena, Options, format_html_with_plugins, parse_document};
use mnemograph_core::title_key;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// Tooltip of links to stubs; the stylesheet also uses it to mark them.
pub const STUB_TOOLTIP: &str = "Not written yet";

/// Characters kept unencoded in page paths.
const PATH_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// The URL of a page or stub in the web UI.
pub fn page_url(title: &str) -> String {
    format!("/pages/{}", utf8_percent_encode(title, PATH_SAFE))
}

/// Rendered HTML and whether it needs the Mermaid script.
pub struct Rendered {
    pub html: String,
    pub has_mermaid: bool,
}

/// Renders Markdown. `stubs` holds the title keys of link targets without a page.
pub fn render(markdown: &str, stubs: &HashSet<String>) -> Rendered {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;
    options.extension.alerts = true;
    options.extension.wikilinks_title_after_pipe = true;
    // Never render raw HTML from bodies; comrak's default, stated explicitly.
    options.render.r#unsafe = false;

    let arena = Arena::new();
    let root = parse_document(&arena, markdown, &options);
    let mut has_mermaid = false;
    for node in root.descendants() {
        let mut data = node.data_mut();
        match &data.value {
            // Wikilinks become ordinary links to the page (or stub) in the web UI.
            NodeValue::WikiLink(link) => {
                let stub = stubs.contains(&title_key(&link.url));
                data.value = NodeValue::Link(Box::new(NodeLink {
                    url: page_url(link.url.trim()),
                    title: if stub {
                        STUB_TOOLTIP.to_owned()
                    } else {
                        String::new()
                    },
                }));
            }
            NodeValue::CodeBlock(block) if block.info.trim() == "mermaid" => has_mermaid = true,
            _ => {}
        }
    }

    let mermaid = MermaidRenderer;
    let mut plugins = Plugins::default();
    plugins.render.codefence_renderers = HashMap::from([(
        "mermaid".to_owned(),
        &mermaid as &dyn CodefenceRendererAdapter,
    )]);
    let mut html = String::new();
    format_html_with_plugins(root, &options, &mut html, &plugins).expect("writing to a String");
    Rendered { html, has_mermaid }
}

/// Renders ```` ```mermaid ```` blocks as `<pre class="mermaid">`, which mermaid.js turns into a
/// diagram in the browser. The diagram source stays HTML-escaped.
struct MermaidRenderer;

impl CodefenceRendererAdapter for MermaidRenderer {
    fn write(
        &self,
        output: &mut dyn Write,
        _lang: &str,
        _meta: &str,
        code: &str,
        _sourcepos: Option<Sourcepos>,
    ) -> fmt::Result {
        output.write_str("<pre class=\"mermaid\">")?;
        output.write_str(&escape_html(code))?;
        output.write_str("</pre>\n")
    }
}

fn escape_html(text: &str) -> Cow<'_, str> {
    if !text.contains(['<', '>', '&', '"', '\'']) {
        return Cow::Borrowed(text);
    }
    let mut escaped = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match c {
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    Cow::Owned(escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(markdown: &str) -> String {
        render(markdown, &HashSet::from(["caddy".to_owned()])).html
    }

    #[test]
    fn wikilinks_point_to_pages_and_mark_stubs() {
        let out = html("[[Rauthy (IdP)|the IdP]] and [[Caddy]]");
        assert!(
            out.contains("<a href=\"/pages/Rauthy%20%28IdP%29\">the IdP</a>"),
            "{out}"
        );
        assert!(
            out.contains("<a href=\"/pages/Caddy\" title=\"Not written yet\">Caddy</a>"),
            "{out}"
        );
    }

    #[test]
    fn raw_html_and_script_links_are_not_rendered() {
        let out = html(
            "<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n\n[click](javascript:alert(1))",
        );
        assert!(!out.contains("<script"), "{out}");
        assert!(!out.contains("<img"), "{out}");
        assert!(!out.contains("javascript:"), "{out}");
    }

    #[test]
    fn mermaid_blocks_become_escaped_diagram_sources() {
        let rendered = render(
            "```mermaid\nflowchart TD\n  A[\"<b>x</b>\"] --> B\n```\n\n```rust\nfn main() {}\n```",
            &HashSet::new(),
        );
        assert!(rendered.has_mermaid);
        assert!(
            rendered.html.contains(
                "<pre class=\"mermaid\">flowchart TD\n  A[&quot;&lt;b&gt;x&lt;/b&gt;&quot;] --&gt; B\n</pre>"
            ),
            "{}",
            rendered.html
        );
        assert!(rendered.html.contains("<code class=\"language-rust\">"));
        assert!(!render("plain", &HashSet::new()).has_mermaid);
    }

    #[test]
    fn extensions_render() {
        let out = html(
            "> [!warning]\n> Careful\n\n- [x] done\n\n| a |\n|---|\n| b |\n\nNote[^1]\n\n[^1]: Footnote",
        );
        assert!(out.contains("markdown-alert-warning"), "{out}");
        assert!(out.contains("type=\"checkbox\""), "{out}");
        assert!(out.contains("<table>"), "{out}");
        assert!(out.contains("footnote"), "{out}");
    }

    #[test]
    fn page_urls_encode_special_characters() {
        assert_eq!(page_url("C++ / Rust"), "/pages/C%2B%2B%20%2F%20Rust");
        assert_eq!(page_url("Größe"), "/pages/Gr%C3%B6%C3%9Fe");
    }
}
