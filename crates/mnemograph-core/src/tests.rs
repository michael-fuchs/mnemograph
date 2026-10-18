use rusqlite::Connection;

use super::*;

fn knowledge() -> Knowledge {
    Knowledge::open_in_memory().unwrap()
}

fn create(knowledge: &Knowledge, title: &str, body: &str) -> Page {
    knowledge
        .write_page(title, body, None, PageChanges::default())
        .unwrap()
}

fn update(knowledge: &Knowledge, title: &str, body: &str, base: u32) -> Result<Page> {
    knowledge.write_page(title, body, Some(base), PageChanges::default())
}

fn link(title: &str, page: Option<&str>) -> LinkTarget {
    LinkTarget {
        title: title.to_owned(),
        page: page.map(str::to_owned),
    }
}

/// Creates a database at schema `version`, lets `fill` add rows, and returns its path.
fn database_at_version(dir: &tempfile::TempDir, version: usize, fill: &str) -> std::path::PathBuf {
    let mut conn = Connection::open_in_memory().unwrap();
    schema::migrations().to_version(&mut conn, version).unwrap();
    conn.execute_batch(fill).unwrap();
    let path = dir.path().join(format!("v{version}.db"));
    conn.backup(rusqlite::MAIN_DB, &path, None).unwrap();
    path
}

// Pages and revisions

#[test]
fn created_page_can_be_read_back() {
    let knowledge = knowledge();
    let created = create(&knowledge, "Rauthy", "An OIDC provider.");
    assert_eq!(created.title, "Rauthy");
    assert_eq!(created.revision, 1);
    assert_eq!(knowledge.page("Rauthy").unwrap().unwrap(), created);
}

#[test]
fn missing_page_is_none() {
    assert_eq!(knowledge().page("Nothing").unwrap(), None);
}

#[test]
fn titles_breaking_link_syntax_are_rejected() {
    let knowledge = knowledge();
    for title in ["", "   ", "a|b", "[[x]]", "two\nlines"] {
        let err = knowledge
            .write_page(title, "", None, PageChanges::default())
            .unwrap_err();
        assert!(matches!(err, Error::InvalidTitle(_)), "{title:?}");
    }
}

#[test]
fn update_with_current_base_creates_a_new_revision_and_keeps_the_old_one() {
    let knowledge = knowledge();
    create(&knowledge, "Caddy", "v1");
    let updated = update(&knowledge, "Caddy", "v2", 1).unwrap();
    assert_eq!((updated.revision, updated.body.as_str()), (2, "v2"));
    assert_eq!(knowledge.page("Caddy").unwrap().unwrap(), updated);

    let first = knowledge.page_revision("Caddy", 1).unwrap().unwrap();
    assert_eq!(first.body, "v1");
    assert_eq!(first.created_at, updated.created_at);
}

#[test]
fn update_without_base_is_a_conflict() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "first");
    let err = knowledge
        .write_page("Rust", "second", None, PageChanges::default())
        .unwrap_err();
    let Error::Conflict { base, current } = err else {
        panic!("expected a conflict, got {err:?}");
    };
    assert_eq!(base, None);
    assert_eq!((current.revision, current.body.as_str()), (1, "first"));
}

#[test]
fn update_with_stale_base_is_a_conflict_and_changes_nothing() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "v1");
    update(&knowledge, "Rust", "v2 by agent A", 1).unwrap();

    let err = update(&knowledge, "Rust", "v2 by agent B", 1).unwrap_err();
    let Error::Conflict { base, current } = err else {
        panic!("expected a conflict, got {err:?}");
    };
    assert_eq!(base, Some(1));
    assert_eq!(
        (current.revision, current.body.as_str()),
        (2, "v2 by agent A")
    );
    assert_eq!(knowledge.page("Rust").unwrap().unwrap().revision, 2);
}

#[test]
fn base_revision_for_a_missing_page_is_rejected() {
    let err = update(&knowledge(), "Nothing", "x", 1).unwrap_err();
    assert!(matches!(err, Error::PageNotFound(_)));
}

#[test]
fn unchanged_write_creates_no_revision() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "same");
    assert_eq!(update(&knowledge, "Rust", "same", 1).unwrap().revision, 1);
}

#[test]
fn data_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mnemograph.db");
    create(
        &Knowledge::open(&path).unwrap(),
        "Caddy",
        "A reverse proxy.",
    );
    let reopened = Knowledge::open(&path).unwrap();
    assert_eq!(
        reopened.page("Caddy").unwrap().unwrap().body,
        "A reverse proxy."
    );
}

// Title normalization, aliases and renames

#[test]
fn titles_are_found_regardless_of_case_and_whitespace() {
    let knowledge = knowledge();
    create(&knowledge, "  Rauthy   IdP ", "");
    let page = knowledge.page("rauthy idp").unwrap().unwrap();
    assert_eq!(page.title, "Rauthy IdP");
}

#[test]
fn creating_a_title_differing_only_in_case_is_a_conflict() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "");
    let err = knowledge
        .write_page("rust", "", None, PageChanges::default())
        .unwrap_err();
    assert!(matches!(err, Error::Conflict { current, .. } if current.title == "Rust"));
}

#[test]
fn aliases_lead_to_the_page() {
    let knowledge = knowledge();
    let page = knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                aliases: Some(vec!["Rust (language)".into(), "rustlang".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.aliases, ["Rust (language)", "rustlang"]);
    assert_eq!(knowledge.page("RUSTLANG").unwrap().unwrap().title, "Rust");
}

#[test]
fn aliases_are_cleaned_deduplicated_and_never_equal_the_title() {
    let knowledge = knowledge();
    let page = knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                aliases: Some(vec![" rustlang ".into(), "RustLang".into(), "rust".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.aliases, ["rustlang"]);
}

#[test]
fn a_name_of_another_page_cannot_become_an_alias() {
    let knowledge = knowledge();
    create(&knowledge, "Caddy", "");
    create(&knowledge, "Traefik", "");
    let err = knowledge
        .write_page(
            "Traefik",
            "",
            Some(1),
            PageChanges {
                aliases: Some(vec!["caddy".into()]),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::NameTaken { name, page } if name == "caddy" && page == "Caddy"));
}

#[test]
fn new_page_cannot_take_an_alias_of_another_page() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                aliases: Some(vec!["rustlang".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    let err = knowledge
        .write_page(
            "Ferris",
            "",
            None,
            PageChanges {
                aliases: Some(vec!["Rustlang".into()]),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::NameTaken { page, .. } if page == "Rust"));
    assert_eq!(knowledge.page("Ferris").unwrap(), None);
}

#[test]
fn rename_keeps_the_old_title_as_alias_and_links_keep_resolving() {
    let knowledge = knowledge();
    create(&knowledge, "Rauthy", "An OIDC provider.");
    create(&knowledge, "Setup", "We use [[Rauthy]].");

    let renamed = knowledge
        .write_page(
            "Rauthy",
            "An OIDC provider.",
            Some(1),
            PageChanges {
                new_title: Some("Rauthy IdP".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(renamed.title, "Rauthy IdP");
    assert_eq!(renamed.aliases, ["Rauthy"]);
    assert_eq!(renamed.revision, 2);

    assert_eq!(
        knowledge.page("Rauthy").unwrap().unwrap().title,
        "Rauthy IdP"
    );
    assert_eq!(
        knowledge.links("Setup").unwrap().outgoing,
        [link("Rauthy", Some("Rauthy IdP"))]
    );
    assert_eq!(knowledge.links("Rauthy IdP").unwrap().backlinks, ["Setup"]);
    assert_eq!(
        knowledge
            .page_revision("Rauthy IdP", 1)
            .unwrap()
            .unwrap()
            .title,
        "Rauthy"
    );
}

#[test]
fn renaming_to_an_own_alias_swaps_title_and_alias() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                aliases: Some(vec!["Rust (language)".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    let renamed = knowledge
        .write_page(
            "Rust",
            "",
            Some(1),
            PageChanges {
                new_title: Some("Rust (language)".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(renamed.title, "Rust (language)");
    assert_eq!(renamed.aliases, ["Rust"]);
}

#[test]
fn case_only_rename_adds_no_alias() {
    let knowledge = knowledge();
    create(&knowledge, "rust", "");
    let renamed = knowledge
        .write_page(
            "rust",
            "",
            Some(1),
            PageChanges {
                new_title: Some("Rust".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!((renamed.title.as_str(), renamed.aliases.len()), ("Rust", 0));
}

#[test]
fn renaming_onto_another_page_is_rejected() {
    let knowledge = knowledge();
    create(&knowledge, "Caddy", "");
    create(&knowledge, "Traefik", "");
    let err = knowledge
        .write_page(
            "Traefik",
            "",
            Some(1),
            PageChanges {
                new_title: Some("CADDY".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::NameTaken { page, .. } if page == "Caddy"));
}

#[test]
fn omitted_aliases_and_type_are_kept() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "v1",
            None,
            PageChanges {
                page_type: Some(Some("technology".into())),
                aliases: Some(vec!["rustlang".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    let page = update(&knowledge, "Rust", "v2", 1).unwrap();
    assert_eq!(page.page_type.as_deref(), Some("technology"));
    assert_eq!(page.aliases, ["rustlang"]);
}

// Page types

#[test]
fn page_type_is_stored_normalized() {
    let knowledge = knowledge();
    let page = knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                page_type: Some(Some(" Technology ".into())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.page_type.as_deref(), Some("technology"));
}

#[test]
fn unknown_page_type_is_rejected_with_the_allowed_types() {
    let err = knowledge()
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                page_type: Some(Some("crate".into())),
                ..Default::default()
            },
        )
        .unwrap_err();
    let message = err.to_string();
    assert!(matches!(err, Error::UnknownPageType { .. }));
    assert!(
        message.contains("project, technology, decision, person, tool, concept"),
        "{message}"
    );
}

#[test]
fn configured_page_types_replace_the_defaults() {
    let knowledge = knowledge().with_page_types(["Library".to_owned()]);
    let changes = |page_type: &str| PageChanges {
        page_type: Some(Some(page_type.into())),
        ..Default::default()
    };
    knowledge
        .write_page("serde", "", None, changes("library"))
        .unwrap();
    assert!(matches!(
        knowledge.write_page("Rust", "", None, changes("technology")),
        Err(Error::UnknownPageType { .. })
    ));
}

#[test]
fn page_type_can_be_removed_and_change_creates_a_revision() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                page_type: Some(Some("tool".into())),
                ..Default::default()
            },
        )
        .unwrap();
    let page = knowledge
        .write_page(
            "Rust",
            "",
            Some(1),
            PageChanges {
                page_type: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!((page.page_type, page.revision), (None, 2));
}

// Links and stubs

#[test]
fn links_are_extracted_and_unwritten_titles_are_stubs() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "A language.");
    create(
        &knowledge,
        "mnemograph",
        "Written in [[rust]], uses [[Tantivy|a search index]].",
    );

    let outgoing = knowledge.links("mnemograph").unwrap().outgoing;
    assert_eq!(
        outgoing,
        [link("rust", Some("Rust")), link("Tantivy", None)]
    );
    assert!(outgoing[1].is_stub());
    assert_eq!(knowledge.links("Rust").unwrap().backlinks, ["mnemograph"]);
}

#[test]
fn stub_has_backlinks_and_becomes_a_page_when_written() {
    let knowledge = knowledge();
    create(&knowledge, "A", "See [[Caddy]].");
    create(&knowledge, "B", "Also [[caddy]].");

    let stub = knowledge.links("CADDY").unwrap();
    assert!(stub.outgoing.is_empty());
    assert_eq!(stub.backlinks, ["A", "B"]);

    create(&knowledge, "Caddy", "A reverse proxy.");
    assert_eq!(
        knowledge.links("A").unwrap().outgoing,
        [link("Caddy", Some("Caddy"))]
    );
    assert_eq!(knowledge.links("Caddy").unwrap().backlinks, ["A", "B"]);
}

#[test]
fn backlinks_include_links_to_aliases() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                aliases: Some(vec!["rustlang".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    create(&knowledge, "A", "[[Rust]]");
    create(&knowledge, "B", "[[rustlang]]");
    assert_eq!(knowledge.links("Rust").unwrap().backlinks, ["A", "B"]);
}

#[test]
fn new_revision_replaces_links() {
    let knowledge = knowledge();
    create(&knowledge, "Page", "[[Old]] and [[Kept]]");
    update(&knowledge, "Page", "[[Kept]] and [[New]]", 1).unwrap();

    let titles: Vec<_> = knowledge
        .links("Page")
        .unwrap()
        .outgoing
        .into_iter()
        .map(|l| l.title)
        .collect();
    assert_eq!(titles, ["Kept", "New"]);
    assert!(knowledge.links("Old").unwrap().backlinks.is_empty());
}

#[test]
fn links_to_the_page_itself_or_its_aliases_are_ignored() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "[[Rust]] aka [[rustlang]].",
            None,
            PageChanges {
                aliases: Some(vec!["rustlang".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(knowledge.links("Rust").unwrap(), Links::default());
}

// Migrations

#[test]
fn migration_turns_existing_pages_into_first_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let path = database_at_version(
        &dir,
        1,
        "INSERT INTO page (title, body) VALUES ('Old', 'from v1 schema');",
    );
    let page = Knowledge::open(&path)
        .unwrap()
        .page("Old")
        .unwrap()
        .unwrap();
    assert_eq!(
        (page.title.as_str(), page.revision, page.body.as_str()),
        ("Old", 1, "from v1 schema")
    );
}

#[test]
fn migration_fills_title_keys_and_extracts_links_of_current_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let path = database_at_version(
        &dir,
        2,
        "INSERT INTO page (id, title) VALUES (1, 'Old'), (2, 'Rust');
         INSERT INTO revision (page_id, number, body) VALUES
             (1, 1, '[[Gone]]'), (1, 2, '[[rust]] and [[Stub]]'), (2, 1, '');",
    );
    let knowledge = Knowledge::open(&path).unwrap();
    assert_eq!(knowledge.page("OLD").unwrap().unwrap().title, "Old");
    assert_eq!(
        knowledge.links("Old").unwrap().outgoing,
        [link("rust", Some("Rust")), link("Stub", None)]
    );
}

#[test]
fn migration_refuses_titles_that_collide_after_normalization() {
    let dir = tempfile::tempdir().unwrap();
    let path = database_at_version(
        &dir,
        3,
        "INSERT INTO page (id, title) VALUES (1, 'Rust'), (2, 'rust');
         INSERT INTO revision (page_id, number, body) VALUES (1, 1, ''), (2, 1, '');",
    );
    let err = Knowledge::open(&path).err().expect("migration must fail");
    assert!(err.to_string().contains("differ only in case"), "{err}");
}
