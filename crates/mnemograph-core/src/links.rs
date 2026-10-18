//! Extraction of links from Markdown bodies.

use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};

/// Returns the distinct link targets in `body` (`[[Title]]` or `[[Title|label]]`), in order of
/// first appearance. Links inside code are not links.
pub(crate) fn link_targets(body: &str) -> Vec<String> {
    let mut options = Options::default();
    options.extension.wikilinks_title_after_pipe = true;
    let arena = Arena::new();
    let root = parse_document(&arena, body, &options);

    let mut targets: Vec<String> = Vec::new();
    for node in root.descendants() {
        if let NodeValue::WikiLink(link) = &node.data().value
            && let Ok(title) = crate::validate_title(&link.url)
            && !targets.iter().any(|t| t == title)
        {
            targets.push(title.to_owned());
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::link_targets;

    #[test]
    fn plain_and_labelled_links() {
        assert_eq!(
            link_targets("Uses [[Rust]] and [[Rauthy (IdP)|the identity provider]]."),
            ["Rust", "Rauthy (IdP)"]
        );
    }

    #[test]
    fn duplicates_keep_first_position() {
        assert_eq!(link_targets("[[B]], [[A]], [[B|again]]"), ["B", "A"]);
    }

    #[test]
    fn links_in_code_are_ignored() {
        let body = "Inline `[[NotALink]]`.\n\n```\n[[AlsoNot]]\n```\n\n[[Real]]";
        assert_eq!(link_targets(body), ["Real"]);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(link_targets("[[ Caddy ]]"), ["Caddy"]);
    }

    #[test]
    fn umlauts_and_spaces_survive() {
        assert_eq!(link_targets("[[Größere Datenbank]]"), ["Größere Datenbank"]);
    }
}
