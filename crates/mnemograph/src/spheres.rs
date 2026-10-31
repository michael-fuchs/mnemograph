//! The spheres file: mnemograph's part of the central configuration of the owner's assistant.
//!
//! The file is generated from the central configuration and mounted read-only. Its format is
//! the central configuration's (version 1): a table `spheres` with one table per sphere. Of the
//! sphere rules, mnemograph applies `wiki`; keys it does not use, such as the rule `llm`, are
//! ignored.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, bail};
use mnemograph_core::{Spheres, is_sphere_name};
use serde::Deserialize;

/// The versions of the format this mnemograph reads.
const SUPPORTED_VERSION: i64 = 1;

/// The configured spheres.
#[derive(Debug, Clone)]
pub struct SphereConfig {
    /// Every configured sphere by name.
    pub spheres: BTreeMap<String, Sphere>,
}

/// One configured sphere.
#[derive(Debug, Clone, Deserialize)]
pub struct Sphere {
    /// What the sphere is about, for people.
    pub description: Option<String>,
    #[serde(default)]
    pub rules: Rules,
}

/// The sphere rules mnemograph applies.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rules {
    /// Whether the sphere's content may enter the wiki.
    #[serde(default)]
    pub wiki: WikiRule,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WikiRule {
    #[default]
    Allowed,
    /// The sphere's content must not enter the wiki.
    Never,
}

#[derive(Deserialize)]
struct File {
    version: i64,
    #[serde(default)]
    spheres: BTreeMap<String, Sphere>,
}

impl SphereConfig {
    /// Reads and checks the spheres file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading the spheres file {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in the spheres file {}", path.display()))
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let file: File = toml::from_str(text)?;
        if file.version != SUPPORTED_VERSION {
            bail!(
                "version {} is not supported; this mnemograph reads version {SUPPORTED_VERSION}",
                file.version
            );
        }
        if file.spheres.is_empty() {
            bail!("no spheres are configured");
        }
        if let Some(name) = file.spheres.keys().find(|name| !is_sphere_name(name)) {
            bail!(
                "invalid sphere name {name:?}: lowercase letters, digits and -, starting with a letter, at most 32 characters"
            );
        }
        Ok(Self {
            spheres: file.spheres,
        })
    }

    /// The names of all configured spheres.
    pub fn names(&self) -> Spheres {
        Spheres::new(self.spheres.keys().cloned()).expect("checked when loading")
    }

    /// The spheres whose content may enter the wiki, if any.
    pub fn allowed(&self) -> Option<Spheres> {
        Spheres::new(
            self.spheres
                .iter()
                .filter(|(_, sphere)| sphere.rules.wiki == WikiRule::Allowed)
                .map(|(name, _)| name.clone()),
        )
        .ok()
    }

    /// The spheres whose rule keeps their content out of the wiki.
    pub fn forbidden(&self) -> Vec<String> {
        self.spheres
            .iter()
            .filter(|(_, sphere)| sphere.rules.wiki == WikiRule::Never)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_rendered_part_of_the_central_configuration() {
        let config = SphereConfig::parse(include_str!("../../../dev/spheres.toml")).unwrap();
        assert_eq!(config.names().names(), ["private", "work"]);
        assert!(config.spheres["work"].description.is_some());
    }

    #[test]
    fn reads_the_wiki_rule() {
        let config = SphereConfig::parse(
            r#"
            version = 1
            [spheres.work]
            rules = { wiki = "never", llm = ["mistral"] }
            [spheres.private]
            rules = { wiki = "allowed" }
            [spheres.clubs]
            "#,
        )
        .unwrap();
        assert_eq!(config.allowed().unwrap().names(), ["clubs", "private"]);
        assert_eq!(config.forbidden(), ["work"]);

        let only_work =
            SphereConfig::parse("version = 1\n[spheres.work.rules]\nwiki = \"never\"\n").unwrap();
        assert!(only_work.allowed().is_none());
        let err = SphereConfig::parse("version = 1\n[spheres.work.rules]\nwiki = \"maybe\"\n")
            .unwrap_err();
        assert!(format!("{err:#}").contains("maybe"), "{err:#}");
    }

    #[test]
    fn ignores_keys_it_does_not_use() {
        let config = SphereConfig::parse(
            r#"
            version = 1
            [spheres.work]
            description = "Employer"
            future = true
            [spheres.work.rules]
            llm = ["mistral"]
            later = "a rule from a newer configuration"
            "#,
        )
        .unwrap();
        assert_eq!(config.names().names(), ["work"]);
    }

    #[test]
    fn rejects_other_versions_missing_spheres_and_invalid_names() {
        for (text, expected) in [
            (
                "version = 2\n[spheres.work]\n",
                "version 2 is not supported",
            ),
            ("version = 1\n", "no spheres"),
            (
                "version = 1\n[spheres.Work]\n",
                "invalid sphere name \"Work\"",
            ),
            ("[spheres.work]\n", "version"),
        ] {
            let err = format!("{:#}", SphereConfig::parse(text).unwrap_err());
            assert!(err.contains(expected), "{text:?}: {err}");
        }
    }
}
