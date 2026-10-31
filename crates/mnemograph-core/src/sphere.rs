//! Spheres: areas of the owner's life with their own privacy boundary (ADR 0008).
//!
//! Every page and note carries one or more spheres. An agent's registration is bound to a set
//! of spheres; through it, the agent sees only what shares a sphere with that set, and what it
//! creates gets one of them.

use crate::{Author, Error, Result};

/// The longest sphere name, as in the central configuration's format.
const MAX_NAME_CHARS: usize = 32;

/// True if `name` is a valid sphere name: lowercase letters, digits and `-`, starting with a
/// letter, at most 32 characters.
pub fn is_sphere_name(name: &str) -> bool {
    name.len() <= MAX_NAME_CHARS
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A non-empty set of valid sphere names, sorted. Only [`Spheres::new`] creates one, so the
/// names are always valid, which lets [`Visibility`] write them into SQL as literals.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spheres(Vec<String>);

impl Spheres {
    /// Checks the names; fails if one is invalid or there is none.
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Result<Self> {
        let mut names: Vec<String> = names.into_iter().map(Into::into).collect();
        if let Some(invalid) = names.iter().find(|name| !is_sphere_name(name)) {
            return Err(Error::InvalidSphere(invalid.clone()));
        }
        if names.is_empty() {
            return Err(Error::NoSphere);
        }
        names.sort();
        names.dedup();
        Ok(Self(names))
    }

    pub fn names(&self) -> &[String] {
        &self.0
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.iter().any(|n| n == name)
    }

    /// The names as an SQL list of string literals, e.g. `'private', 'work'`. Safe because
    /// valid names contain no quotes.
    fn sql_list(&self) -> String {
        self.0
            .iter()
            .inspect(|name| assert!(is_sphere_name(name), "unchecked sphere name"))
            .map(|name| format!("'{name}'"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The sphere something new gets: the one named, which must be among these, or the only
    /// one if none is named.
    pub fn choose(&self, named: Option<&str>) -> Result<&str> {
        match (named, self.0.as_slice()) {
            (Some(named), _) => self
                .0
                .iter()
                .find(|n| *n == named)
                .map(String::as_str)
                .ok_or_else(|| Error::SphereNotAllowed {
                    given: named.to_owned(),
                    allowed: self.0.clone(),
                }),
            (None, [only]) => Ok(only),
            (None, _) => Err(Error::SphereRequired(self.0.clone())),
        }
    }
}

/// Which pages and notes a caller sees.
#[derive(Debug, Clone, Copy)]
pub enum Visibility<'a> {
    /// Everything: the owner.
    All,
    /// Only what shares a sphere with these: an agent through its registration.
    Within(&'a Spheres),
}

impl Visibility<'_> {
    /// True if something carrying `spheres` is visible.
    pub fn sees(&self, spheres: &[String]) -> bool {
        match self {
            Visibility::All => true,
            Visibility::Within(allowed) => spheres.iter().any(|s| allowed.contains(s)),
        }
    }

    /// An SQL condition that holds if the page ID in `column` is visible.
    pub(crate) fn pages(&self, column: &str) -> String {
        self.condition(column, "SELECT page_id FROM page_sphere")
    }

    /// An SQL condition that holds if the note ID in `column` is visible.
    pub(crate) fn notes(&self, column: &str) -> String {
        self.condition(column, "SELECT note_id FROM note_sphere")
    }

    /// An SQL condition that holds if the access log entry ID in `column` was made through a
    /// registration sharing a sphere with these; entries from before spheres have none.
    pub(crate) fn accesses(&self, column: &str) -> String {
        self.condition(column, "SELECT access_id FROM access_sphere")
    }

    /// `column IN (ids of rows of a sphere table in these spheres)`, or true for everything.
    fn condition(&self, column: &str, ids: &str) -> String {
        match self {
            Visibility::All => "1".to_owned(),
            Visibility::Within(spheres) => format!(
                "{column} IN ({ids} WHERE sphere IN ({}))",
                spheres.sql_list()
            ),
        }
    }

    /// Every key that leads to a visible page: its title key and its aliases' keys. Names of
    /// other pages lead nowhere, so links to them are stubs.
    pub(crate) fn resolved_keys(&self) -> String {
        format!(
            "SELECT title_key AS key, id AS page_id FROM page WHERE {} \
             UNION ALL SELECT key, page_id FROM alias WHERE {}",
            self.pages("id"),
            self.pages("page_id")
        )
    }
}

/// Who writes: the author recorded for the change and the spheres of their registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    pub author: Author,
    pub spheres: Spheres,
}

impl Writer {
    pub fn visibility(&self) -> Visibility<'_> {
        Visibility::Within(&self.spheres)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_names_follow_the_configuration_format() {
        for valid in ["work", "private", "club-2", &"a".repeat(32)] {
            assert!(is_sphere_name(valid), "{valid:?}");
        }
        for invalid in [
            "",
            "Work",
            "2nd",
            "-x",
            "a b",
            "sphere:work",
            &"a".repeat(33),
        ] {
            assert!(!is_sphere_name(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn sphere_sets_are_sorted_deduplicated_and_never_empty() {
        let spheres = Spheres::new(["work", "private", "work"]).unwrap();
        assert_eq!(spheres.names(), ["private", "work"]);
        assert!(matches!(
            Spheres::new(Vec::<String>::new()),
            Err(Error::NoSphere)
        ));
        assert!(matches!(
            Spheres::new(["work", "Private"]),
            Err(Error::InvalidSphere(name)) if name == "Private"
        ));
    }

    #[test]
    fn a_single_sphere_is_chosen_by_default_several_must_be_named() {
        let one = Spheres::new(["work"]).unwrap();
        assert_eq!(one.choose(None).unwrap(), "work");
        assert_eq!(one.choose(Some("work")).unwrap(), "work");
        assert!(matches!(
            one.choose(Some("private")),
            Err(Error::SphereNotAllowed { .. })
        ));

        let two = Spheres::new(["work", "private"]).unwrap();
        assert_eq!(two.choose(Some("private")).unwrap(), "private");
        let err = two.choose(None).unwrap_err();
        assert!(matches!(err, Error::SphereRequired(_)));
        assert!(err.to_string().contains("private, work"), "{err}");
    }

    #[test]
    fn visibility_needs_one_shared_sphere() {
        let work = Spheres::new(["work"]).unwrap();
        let within = Visibility::Within(&work);
        assert!(within.sees(&["work".into()]));
        assert!(within.sees(&["private".into(), "work".into()]));
        assert!(!within.sees(&["private".into()]));
        assert!(Visibility::All.sees(&["private".into()]));
    }
}
