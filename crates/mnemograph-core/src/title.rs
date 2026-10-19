//! Titles: validation and the normalized key used to compare them.

use crate::{Error, Result};

/// Characters that would break the `[[Title|label]]` link syntax.
const FORBIDDEN_TITLE_CHARS: &[char] = &['[', ']', '|'];

/// Validates a title or alias and returns it with surrounding whitespace removed and inner
/// whitespace collapsed to single spaces.
pub(crate) fn clean_title(title: &str) -> Result<String> {
    let cleaned = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() || cleaned.contains(FORBIDDEN_TITLE_CHARS) || title.contains(['\n', '\r'])
    {
        return Err(Error::InvalidTitle(title.to_owned()));
    }
    Ok(cleaned)
}

/// The key under which titles and aliases are compared: two titles with the same key name the
/// same page.
pub fn title_key(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning_collapses_whitespace() {
        assert_eq!(clean_title("  Rauthy \t (IdP) ").unwrap(), "Rauthy (IdP)");
    }

    #[test]
    fn invalid_titles() {
        for title in ["", "   ", "a|b", "[[x]]", "two\nlines"] {
            assert!(clean_title(title).is_err(), "{title:?}");
        }
    }

    #[test]
    fn keys_ignore_case_and_whitespace() {
        assert_eq!(title_key(" Rauthy  IdP "), title_key("rauthy idp"));
        assert_eq!(title_key("GRÖSSE"), "grösse");
    }
}
