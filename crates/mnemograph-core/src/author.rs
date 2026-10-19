//! Authors: who created a revision or note.

use std::fmt;

/// The owner or agent that created a revision or note, as established by authentication —
/// never by anything a request claims about itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Author {
    /// The owner, identified by their user ID at the identity provider.
    Owner(String),
    /// An agent, identified by the OAuth client ID of its installation.
    Agent(String),
    /// Someone unknown, because authentication is disabled (local development only).
    Anonymous,
}

impl Author {
    /// Parses the stored form written by [`Author`]'s `Display`; unknown forms are `None`.
    pub fn parse(stored: &str) -> Option<Self> {
        match stored.split_once(':') {
            Some(("owner", id)) => Some(Author::Owner(id.to_owned())),
            Some(("agent", id)) => Some(Author::Agent(id.to_owned())),
            None if stored == "anonymous" => Some(Author::Anonymous),
            _ => None,
        }
    }
}

impl fmt::Display for Author {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Author::Owner(id) => write!(f, "owner:{id}"),
            Author::Agent(id) => write!(f, "agent:{id}"),
            Author::Anonymous => f.write_str("anonymous"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Author;

    #[test]
    fn stored_form_round_trips() {
        for author in [
            Author::Owner("u-1".into()),
            Author::Agent("claude-code:laptop".into()),
            Author::Anonymous,
        ] {
            assert_eq!(Author::parse(&author.to_string()), Some(author));
        }
        assert_eq!(Author::parse("robot:x"), None);
    }
}
