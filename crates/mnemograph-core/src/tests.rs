use rusqlite::Connection;

use super::*;

fn author() -> Author {
    Author::Agent("test-agent".into())
}

fn knowledge() -> Knowledge {
    Knowledge::open_in_memory().unwrap()
}

fn create(knowledge: &Knowledge, title: &str, body: &str) -> Page {
    knowledge
        .write_page(title, body, None, PageChanges::default(), &author())
        .unwrap()
}

fn update(knowledge: &Knowledge, title: &str, body: &str, base: u32) -> Result<Page> {
    knowledge.write_page(title, body, Some(base), PageChanges::default(), &author())
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
            .write_page(title, "", None, PageChanges::default(), &author())
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
        .write_page("Rust", "second", None, PageChanges::default(), &author())
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
        .write_page("rust", "", None, PageChanges::default(), &author())
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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
        .write_page("serde", "", None, changes("library"), &author())
        .unwrap();
    assert!(matches!(
        knowledge.write_page("Rust", "", None, changes("technology"), &author()),
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
            &author(),
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
            &author(),
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
            &author(),
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
            &author(),
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

// Notes

#[test]
fn note_links_attach_it_to_pages_and_stubs() {
    let knowledge = knowledge();
    create(&knowledge, "Reverse proxy", "");
    let note = knowledge
        .add_note(
            "Decided [[reverse proxy|the proxy]] is [[Caddy]] because of automatic HTTPS.",
            &author(),
        )
        .unwrap();
    assert!(note.created_at.ends_with('Z'));
    assert_eq!(
        note.links,
        [
            link("reverse proxy", Some("Reverse proxy")),
            link("Caddy", None)
        ]
    );

    assert_eq!(
        knowledge.notes_about("Reverse Proxy", 10).unwrap(),
        std::slice::from_ref(&note)
    );
    assert_eq!(knowledge.notes_about("caddy", 10).unwrap(), [note]);
}

#[test]
fn notes_about_a_page_include_links_to_its_aliases_newest_first() {
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
            &author(),
        )
        .unwrap();
    let first = knowledge
        .add_note("[[Rust]] 1.99 is out.", &author())
        .unwrap();
    knowledge
        .add_note("Unrelated [[Caddy]] note.", &author())
        .unwrap();
    let second = knowledge
        .add_note("[[rustlang]] and [[Rust]] again.", &author())
        .unwrap();

    let notes = knowledge.notes_about("Rust", 10).unwrap();
    assert_eq!(
        notes.iter().map(|n| n.id).collect::<Vec<_>>(),
        [second.id, first.id]
    );
    assert_eq!(knowledge.notes_about("Rust", 1).unwrap().len(), 1);
}

#[test]
fn notes_attached_to_a_stub_stay_attached_when_it_becomes_a_page() {
    let knowledge = knowledge();
    let note = knowledge
        .add_note("Look into [[Tantivy]].", &author())
        .unwrap();
    create(&knowledge, "Tantivy", "A search library.");
    let notes = knowledge.notes_about("Tantivy", 10).unwrap();
    assert_eq!(notes[0].id, note.id);
    assert_eq!(notes[0].links, [link("Tantivy", Some("Tantivy"))]);
}

#[test]
fn notes_without_links_are_allowed_but_empty_notes_are_not() {
    let knowledge = knowledge();
    assert!(
        knowledge
            .add_note("Just a thought.", &author())
            .unwrap()
            .links
            .is_empty()
    );
    assert!(matches!(
        knowledge.add_note("  \n ", &author()),
        Err(Error::EmptyNote)
    ));
}

#[test]
fn notes_are_not_backlinks() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "");
    knowledge.add_note("[[Rust]]", &author()).unwrap();
    assert!(knowledge.links("Rust").unwrap().backlinks.is_empty());
}

// Related pages

fn related(knowledge: &Knowledge, title: &str, hops: u32) -> Vec<(String, u32, bool)> {
    let mut found: Vec<_> = knowledge
        .related(title, hops, 100)
        .unwrap()
        .into_iter()
        .map(|r| (r.title, r.distance, r.stub))
        .collect();
    found.sort();
    found
}

fn r(title: &str, distance: u32, stub: bool) -> (String, u32, bool) {
    (title.to_owned(), distance, stub)
}

#[test]
fn related_follows_links_in_both_directions_up_to_the_hop_count() {
    let knowledge = knowledge();
    // A → B → C → D, and E → A
    create(&knowledge, "A", "[[B]]");
    create(&knowledge, "B", "[[C]]");
    create(&knowledge, "C", "[[D]]");
    create(&knowledge, "D", "");
    create(&knowledge, "E", "[[A]]");

    assert_eq!(
        related(&knowledge, "A", 1),
        [r("B", 1, false), r("E", 1, false)]
    );
    assert_eq!(
        related(&knowledge, "A", 3),
        [
            r("B", 1, false),
            r("C", 2, false),
            r("D", 3, false),
            r("E", 1, false)
        ]
    );
}

#[test]
fn related_reports_the_shortest_distance_in_cycles() {
    let knowledge = knowledge();
    create(&knowledge, "A", "[[B]] [[C]]");
    create(&knowledge, "B", "[[C]] [[A]]");
    create(&knowledge, "C", "[[A]]");
    assert_eq!(
        related(&knowledge, "A", 3),
        [r("B", 1, false), r("C", 1, false)]
    );
}

#[test]
fn related_resolves_aliases_and_includes_stubs() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                aliases: Some(vec!["rustlang".into()]),
                page_type: Some(Some("technology".into())),
                ..Default::default()
            },
            &author(),
        )
        .unwrap();
    create(&knowledge, "mnemograph", "[[rustlang]] and [[Tantivy]]");
    create(&knowledge, "Ferris", "[[RUST]]");

    assert_eq!(
        related(&knowledge, "rustlang", 2),
        [
            r("Ferris", 1, false),
            r("Tantivy", 2, true),
            r("mnemograph", 1, false)
        ]
    );
    let rust = &knowledge.related("mnemograph", 1, 100).unwrap()[0];
    assert_eq!(rust.page_type.as_deref(), Some("technology"));
}

#[test]
fn related_works_from_a_stub_but_not_from_an_unknown_title() {
    let knowledge = knowledge();
    create(&knowledge, "A", "[[Caddy]]");
    create(&knowledge, "B", "[[A]]");
    assert_eq!(
        related(&knowledge, "caddy", 2),
        [r("A", 1, false), r("B", 2, false)]
    );
    assert!(matches!(
        knowledge.related("Nothing", 2, 100),
        Err(Error::PageNotFound(_))
    ));
}

#[test]
fn related_clamps_hops_and_respects_the_limit() {
    let knowledge = knowledge();
    create(&knowledge, "A", "[[B]]");
    create(&knowledge, "B", "[[C]]");
    create(&knowledge, "C", "[[D]]");
    create(&knowledge, "D", "[[E]]");
    assert_eq!(related(&knowledge, "A", 0).len(), 1);
    assert_eq!(related(&knowledge, "A", 99).len(), 3);
    assert_eq!(knowledge.related("A", 3, 2).unwrap().len(), 2);
}

/// Nodes as (title, distance, stub) and links as sorted title pairs.
type Graph = (Vec<(String, u32, bool)>, Vec<(String, String)>);

/// The neighborhood as titles with stub flags, and its links as sorted title pairs.
fn neighborhood(knowledge: &Knowledge, title: &str, hops: u32) -> Graph {
    let graph = knowledge.neighborhood(title, hops, 100).unwrap();
    let title_of = |i: usize| graph.nodes[i].title.clone();
    let mut edges: Vec<_> = graph
        .edges
        .iter()
        .map(|&(a, b)| {
            let (a, b) = (title_of(a), title_of(b));
            if a < b { (a, b) } else { (b, a) }
        })
        .collect();
    edges.sort();
    let nodes = graph
        .nodes
        .into_iter()
        .map(|n| (n.title, n.distance, n.stub))
        .collect();
    (nodes, edges)
}

fn e(a: &str, b: &str) -> (String, String) {
    (a.to_owned(), b.to_owned())
}

#[test]
fn the_neighborhood_has_the_related_pages_and_the_links_between_them() {
    let knowledge = knowledge();
    // A → B → C → D, E → A, B ↔ E, and a link to the stub Caddy
    create(&knowledge, "A", "[[B]]");
    create(&knowledge, "B", "[[C]] [[E]] [[Caddy]]");
    create(&knowledge, "C", "[[D]]");
    create(&knowledge, "D", "");
    create(&knowledge, "E", "[[A]] [[B]]");

    for hops in 1..=3 {
        let (nodes, _) = neighborhood(&knowledge, "A", hops);
        assert_eq!(nodes[0], r("A", 0, false));
        let mut rest = nodes[1..].to_vec();
        rest.sort();
        assert_eq!(rest, related(&knowledge, "A", hops), "{hops} hops");
    }
    let (_, edges) = neighborhood(&knowledge, "A", 2);
    // C → D is outside two hops; B ↔ E counts once.
    assert_eq!(
        edges,
        [
            e("A", "B"),
            e("A", "E"),
            e("B", "C"),
            e("B", "Caddy"),
            e("B", "E")
        ]
    );
}

#[test]
fn the_neighborhood_resolves_aliases_and_starts_from_stubs() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "[[Rust]]",
            None,
            PageChanges {
                aliases: Some(vec!["rustlang".into()]),
                ..Default::default()
            },
            &author(),
        )
        .unwrap();
    create(&knowledge, "mnemograph", "[[rustlang]] and [[Tantivy]]");

    let (nodes, edges) = neighborhood(&knowledge, "tantivy", 2);
    assert_eq!(
        nodes,
        [
            r("Tantivy", 0, true),
            r("mnemograph", 1, false),
            r("Rust", 2, false)
        ]
    );
    // The self-link of Rust is no edge.
    assert_eq!(edges, [e("Rust", "mnemograph"), e("Tantivy", "mnemograph")]);
    assert!(matches!(
        knowledge.neighborhood("Nothing", 2, 100),
        Err(Error::PageNotFound(_))
    ));
}

/// The whole-wiki graph as (title, degree, stub), its links as sorted title pairs, and the
/// number of hidden nodes.
fn wiki_graph(knowledge: &Knowledge, page_types: &[&str], max_nodes: usize) -> (Graph, usize) {
    let types: Vec<String> = page_types.iter().map(|t| t.to_string()).collect();
    let graph = knowledge.wiki_graph(&types, max_nodes).unwrap();
    let title_of = |i: usize| graph.nodes[i].title.clone();
    let mut edges: Vec<_> = graph
        .edges
        .iter()
        .map(|&(a, b)| {
            let (a, b) = (title_of(a), title_of(b));
            if a < b { (a, b) } else { (b, a) }
        })
        .collect();
    edges.sort();
    let nodes = graph
        .nodes
        .into_iter()
        .map(|n| (n.title, n.degree, n.stub))
        .collect();
    ((nodes, edges), graph.hidden)
}

fn typed(knowledge: &Knowledge, title: &str, page_type: &str, body: &str) {
    knowledge
        .write_page(
            title,
            body,
            None,
            PageChanges {
                page_type: Some(Some(page_type.into())),
                ..Default::default()
            },
            &author(),
        )
        .unwrap();
}

#[test]
fn the_wiki_graph_has_every_page_and_stub_most_linked_first() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rust",
            "[[Rust]]",
            None,
            PageChanges {
                aliases: Some(vec!["rustlang".into()]),
                ..Default::default()
            },
            &author(),
        )
        .unwrap();
    // Links under the title and an alias, in both directions, count as one.
    create(
        &knowledge,
        "mnemograph",
        "[[Rust]] [[rustlang]] [[Tantivy]]",
    );
    create(&knowledge, "Ferris", "[[RUST]] [[mnemograph]]");
    create(&knowledge, "Alone", "");

    let ((nodes, edges), hidden) = wiki_graph(&knowledge, &[], 100);
    assert_eq!(
        nodes,
        [
            r("mnemograph", 3, false),
            r("Ferris", 2, false),
            r("Rust", 2, false),
            r("Tantivy", 1, true),
            r("Alone", 0, false),
        ]
    );
    assert_eq!(
        edges,
        [
            e("Ferris", "Rust"),
            e("Ferris", "mnemograph"),
            e("Rust", "mnemograph"),
            e("Tantivy", "mnemograph")
        ]
    );
    assert_eq!(hidden, 0);
}

#[test]
fn the_wiki_graph_filters_by_page_type_and_keeps_the_most_linked() {
    let knowledge = knowledge();
    typed(
        &knowledge,
        "mnemograph",
        "project",
        "[[Rust]] [[SQLite]] [[Caddy]] [[Stub]]",
    );
    typed(&knowledge, "Rust", "technology", "[[SQLite]]");
    typed(&knowledge, "SQLite", "technology", "");
    typed(&knowledge, "Caddy", "tool", "");

    // Only technologies: no project, no tool, no stubs; links among the shown pages only.
    let ((nodes, edges), hidden) = wiki_graph(&knowledge, &["technology"], 100);
    assert_eq!(nodes, [r("Rust", 1, false), r("SQLite", 1, false)]);
    assert_eq!(edges, [e("Rust", "SQLite")]);
    assert_eq!(hidden, 0);

    let ((nodes, _), _) = wiki_graph(&knowledge, &["technology", "tool"], 100);
    assert_eq!(nodes.len(), 3);

    // Above the limit, the most-linked stay and the rest are counted.
    let ((nodes, edges), hidden) = wiki_graph(&knowledge, &[], 2);
    assert_eq!(nodes, [r("mnemograph", 4, false), r("Rust", 2, false)]);
    assert_eq!(edges, [e("Rust", "mnemograph")]);
    assert_eq!(hidden, 3);
}

#[test]
fn a_page_without_links_has_no_related_pages() {
    let knowledge = knowledge();
    create(&knowledge, "Alone", "");
    assert!(knowledge.related("Alone", 3, 100).unwrap().is_empty());
}

// Search

fn found_titles(hits: &[SearchHit]) -> Vec<String> {
    hits.iter()
        .map(|hit| match &hit.found {
            Found::Page { title, .. } => title.clone(),
            Found::Note(note) => format!("note {}", note.id),
        })
        .collect()
}

fn search(knowledge: &Knowledge, query: &str) -> Vec<String> {
    found_titles(&knowledge.search(query, None, 10).unwrap())
}

#[test]
fn search_stems_german_and_english() {
    let knowledge = knowledge();
    create(
        &knowledge,
        "Speicher",
        "Wir vergleichen mehrere Datenbanken.",
    );
    create(
        &knowledge,
        "Storage",
        "Comparing several databases and indexes.",
    );

    assert_eq!(search(&knowledge, "Datenbank"), ["Speicher"]);
    assert_eq!(search(&knowledge, "database"), ["Storage"]);
    assert_eq!(search(&knowledge, "compared"), ["Storage"]);
}

#[test]
fn search_finds_titles_aliases_and_notes() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rauthy",
            "An identity provider.",
            None,
            PageChanges {
                aliases: Some(vec!["IdP".into()]),
                ..Default::default()
            },
            &author(),
        )
        .unwrap();
    let note = knowledge
        .add_note(
            "Decided on [[Caddy]] because of automatic certificates.",
            &author(),
        )
        .unwrap();

    assert_eq!(search(&knowledge, "rauthy"), ["Rauthy"]);
    assert_eq!(search(&knowledge, "idp"), ["Rauthy"]);
    let hits = knowledge.search("certificate", None, 10).unwrap();
    assert_eq!(found_titles(&hits), [format!("note {}", note.id)]);
    assert!(
        hits[0].snippet.contains("**certificates**"),
        "{}",
        hits[0].snippet
    );
}

#[test]
fn search_reflects_the_current_revision_and_renames() {
    let knowledge = knowledge();
    create(&knowledge, "Proxy", "We use Traefik.");
    knowledge
        .write_page(
            "Proxy",
            "We use Caddy.",
            Some(1),
            PageChanges {
                new_title: Some("Reverse proxy".into()),
                ..Default::default()
            },
            &author(),
        )
        .unwrap();
    assert!(search(&knowledge, "traefik").is_empty());
    assert_eq!(search(&knowledge, "caddy"), ["Reverse proxy"]);
}

#[test]
fn search_filters_by_page_type_and_rejects_unknown_types() {
    let knowledge = knowledge();
    let typed = |title: &str, page_type: &str| {
        knowledge
            .write_page(
                title,
                "About Rust.",
                None,
                PageChanges {
                    page_type: Some(Some(page_type.into())),
                    ..Default::default()
                },
                &author(),
            )
            .unwrap();
    };
    typed("Rust", "technology");
    typed("Use Rust", "decision");
    knowledge.add_note("Rust note", &author()).unwrap();

    let hits = knowledge.search("rust", Some("Decision"), 10).unwrap();
    assert_eq!(found_titles(&hits), ["Use Rust"]);
    assert!(matches!(
        knowledge.search("rust", Some("crate"), 10),
        Err(Error::UnknownPageType { .. })
    ));
}

#[test]
fn search_tolerates_query_syntax_errors() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "A language.");
    assert_eq!(search(&knowledge, "rust AND ("), ["Rust"]);
    assert!(search(&knowledge, "").is_empty());
}

#[test]
fn a_missing_search_index_is_rebuilt_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kb.db");
    create(&Knowledge::open(&path).unwrap(), "Rust", "A language.");
    std::fs::remove_dir_all(dir.path().join("kb.db.search")).unwrap();

    assert_eq!(
        search(&Knowledge::open(&path).unwrap(), "language"),
        ["Rust"]
    );
}

#[test]
fn a_write_the_index_missed_is_detected_and_repaired_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kb.db");
    create(&Knowledge::open(&path).unwrap(), "Rust", "A language.");

    // Simulate a crash between the database commit and the index update.
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "INSERT INTO note (body) VALUES ('Missed by the index');
         UPDATE meta SET value = value + 1 WHERE key = 'change_counter';",
    )
    .unwrap();
    drop(conn);

    assert_eq!(search(&Knowledge::open(&path).unwrap(), "missed").len(), 1);
}

#[test]
fn rebuild_search_index_restores_all_content() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "A language.");
    knowledge
        .add_note("A note about [[Rust]].", &author())
        .unwrap();
    knowledge.rebuild_search_index().unwrap();
    assert_eq!(search(&knowledge, "rust").len(), 2);
}

#[test]
fn snippets_of_long_texts_are_cut_around_the_match() {
    let knowledge = knowledge();
    let body = format!(
        "{} The decisive word is Tantivy. {}",
        "Filler text. ".repeat(40),
        "More filler. ".repeat(40)
    );
    create(&knowledge, "Long", &body);
    let snippet = &knowledge.search("tantivy", None, 1).unwrap()[0].snippet;
    assert!(snippet.contains("**Tantivy**"), "{snippet}");
    assert!(
        snippet.starts_with('…') && snippet.ends_with('…'),
        "{snippet}"
    );
    assert!(snippet.chars().count() < 260, "{snippet}");
}

// Export

#[test]
fn export_writes_pages_as_markdown_and_notes_as_jsonl() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Rauthy (IdP)",
            "An OIDC provider.\n\nSee [[Rust]].",
            None,
            PageChanges {
                page_type: Some(Some("tool".into())),
                aliases: Some(vec!["Rauthy".into(), "Say \"hi\"".into()]),
                ..Default::default()
            },
            &author(),
        )
        .unwrap();
    create(&knowledge, "C++", "no type, no aliases");
    create(&knowledge, "C#", "collides with C++");
    knowledge
        .add_note("Decided on [[Rauthy]].", &author())
        .unwrap();
    knowledge.add_note("Second\nline", &author()).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("export");
    let summary = knowledge.export(&target).unwrap();
    assert_eq!(summary, ExportSummary { pages: 3, notes: 2 });

    let rauthy = std::fs::read_to_string(target.join("pages/Rauthy-IdP.md")).unwrap();
    assert!(rauthy.starts_with(
        "---\ntitle: \"Rauthy (IdP)\"\ntype: \"tool\"\naliases: [\"Rauthy\",\"Say \\\"hi\\\"\"]\nrevision: 1\n"
    ), "{rauthy}");
    assert!(
        rauthy.ends_with("---\n\nAn OIDC provider.\n\nSee [[Rust]].\n"),
        "{rauthy}"
    );

    let c = std::fs::read_to_string(target.join("pages/C.md")).unwrap();
    assert!(c.starts_with("---\ntitle: \"C++\"\nrevision: 1\n"), "{c}");
    assert!(target.join("pages/C-2.md").exists());

    let notes = std::fs::read_to_string(target.join("notes.jsonl")).unwrap();
    let lines: Vec<serde_json::Value> = notes
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["body"], "Decided on [[Rauthy]].");
    assert_eq!(lines[0]["links"], serde_json::json!(["Rauthy"]));
    assert_eq!(lines[1]["body"], "Second\nline");
    assert!(lines[1]["created_at"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn export_refuses_a_non_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("keep.txt"), "mine").unwrap();
    let err = knowledge().export(dir.path()).unwrap_err();
    assert!(matches!(err, Error::ExportTargetNotEmpty(_)));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("keep.txt")).unwrap(),
        "mine"
    );
}

// Deletion

#[test]
fn deleting_a_page_removes_it_and_its_title_becomes_a_stub() {
    let knowledge = knowledge();
    knowledge
        .write_page(
            "Traefik",
            "A proxy with [[Docker]] labels.",
            None,
            PageChanges {
                aliases: Some(vec!["traefik proxy".into()]),
                ..Default::default()
            },
            &author(),
        )
        .unwrap();
    update(&knowledge, "Traefik", "Second revision.", 1).unwrap();
    create(&knowledge, "Proxies", "[[Traefik]] or [[Traefik proxy]]");
    let note = knowledge
        .add_note("Evaluated [[Traefik]].", &author())
        .unwrap();

    let deleted = knowledge.delete_page("traefik proxy").unwrap();
    assert_eq!(
        deleted,
        DeletedPage {
            title: "Traefik".into(),
            aliases: vec!["traefik proxy".into()],
            revisions: 2
        }
    );

    assert_eq!(knowledge.page("Traefik").unwrap(), None);
    assert_eq!(knowledge.page_revision("Traefik", 1).unwrap(), None);
    assert!(
        knowledge
            .links("Proxies")
            .unwrap()
            .outgoing
            .iter()
            .all(LinkTarget::is_stub)
    );
    assert_eq!(knowledge.links("Traefik").unwrap().backlinks, ["Proxies"]);
    assert_eq!(knowledge.notes_about("Traefik", 10).unwrap()[0].id, note.id);
    assert!(knowledge.links("Docker").unwrap().backlinks.is_empty());
    assert!(search(&knowledge, "labels").is_empty());
}

#[test]
fn a_deleted_title_can_be_written_again_from_scratch() {
    let knowledge = knowledge();
    create(&knowledge, "Traefik", "old");
    update(&knowledge, "Traefik", "older", 1).unwrap();
    knowledge.delete_page("Traefik").unwrap();
    assert_eq!(create(&knowledge, "Traefik", "new").revision, 1);
}

#[test]
fn deleting_a_note_removes_only_that_note() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "");
    let first = knowledge
        .add_note("First about [[Rust]], unusualword.", &author())
        .unwrap();
    let second = knowledge
        .add_note("Second about [[Rust]].", &author())
        .unwrap();

    knowledge.delete_note(first.id).unwrap();
    let remaining: Vec<_> = knowledge
        .notes_about("Rust", 10)
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(remaining, [second.id]);
    assert!(search(&knowledge, "unusualword").is_empty());
    assert!(knowledge.page("Rust").unwrap().is_some());
}

#[test]
fn deleting_something_missing_is_an_error() {
    let knowledge = knowledge();
    assert!(matches!(
        knowledge.delete_page("Nothing"),
        Err(Error::PageNotFound(_))
    ));
    assert!(matches!(
        knowledge.delete_note(42),
        Err(Error::NoteNotFound(42))
    ));
}

#[test]
fn deletions_survive_reopening_without_reindexing_stale_hits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kb.db");
    {
        let knowledge = Knowledge::open(&path).unwrap();
        create(&knowledge, "Gone", "vanishing content");
        knowledge.delete_page("Gone").unwrap();
    }
    assert!(search(&Knowledge::open(&path).unwrap(), "vanishing").is_empty());
}

// Authors

#[test]
fn every_revision_and_note_records_its_author() {
    let knowledge = knowledge();
    let claude = Author::Agent("claude-code".into());
    let owner = Author::Owner("user-1".into());
    knowledge
        .write_page("Rust", "v1", None, PageChanges::default(), &claude)
        .unwrap();
    knowledge
        .write_page("Rust", "v2", Some(1), PageChanges::default(), &owner)
        .unwrap();
    let note = knowledge.add_note("About [[Rust]].", &claude).unwrap();

    assert_eq!(knowledge.page("Rust").unwrap().unwrap().author, Some(owner));
    assert_eq!(
        knowledge.page_revision("Rust", 1).unwrap().unwrap().author,
        Some(claude.clone())
    );
    assert_eq!(note.author, Some(claude.clone()));
    assert_eq!(
        knowledge.notes_about("Rust", 1).unwrap()[0].author,
        Some(claude)
    );
}

#[test]
fn rows_from_before_authors_have_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = database_at_version(
        &dir,
        6,
        "INSERT INTO page (id, title, title_key) VALUES (1, 'Old', 'old');
         INSERT INTO revision (page_id, number, title, body) VALUES (1, 1, 'Old', '');
         INSERT INTO note (body) VALUES ('old note');",
    );
    let knowledge = Knowledge::open(&path).unwrap();
    assert_eq!(knowledge.page("Old").unwrap().unwrap().author, None);
    assert_eq!(knowledge.notes().unwrap()[0].author, None);
}

#[test]
fn export_includes_authors() {
    let knowledge = knowledge();
    create(&knowledge, "Rust", "");
    knowledge.add_note("note", &author()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    knowledge.export(&dir.path().join("out")).unwrap();
    let page = std::fs::read_to_string(dir.path().join("out/pages/Rust.md")).unwrap();
    assert!(
        page.contains("updated_by: \"agent:test-agent\"\n"),
        "{page}"
    );
    let notes = std::fs::read_to_string(dir.path().join("out/notes.jsonl")).unwrap();
    assert!(notes.contains("\"author\":\"agent:test-agent\""), "{notes}");
}

// Sessions

#[test]
fn sessions_are_found_by_token_until_they_end() {
    let knowledge = knowledge();
    let owner = Session {
        owner_id: "user-1".into(),
        owner_name: "admin@localhost".into(),
    };
    let token = knowledge
        .create_session(&owner, std::time::Duration::from_secs(3600))
        .unwrap();
    assert!(token.len() >= 43, "256 bits of randomness");
    assert_eq!(knowledge.session(&token).unwrap(), Some(owner));
    assert_eq!(knowledge.session("guessed").unwrap(), None);

    knowledge.end_session(&token).unwrap();
    assert_eq!(knowledge.session(&token).unwrap(), None);
}

#[test]
fn expired_sessions_are_not_found_and_tokens_are_not_stored() {
    let knowledge = knowledge();
    let owner = Session {
        owner_id: "user-1".into(),
        owner_name: "admin".into(),
    };
    let expired = knowledge
        .create_session(&owner, std::time::Duration::from_secs(0))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert_eq!(knowledge.session(&expired).unwrap(), None);

    let token = knowledge
        .create_session(&owner, std::time::Duration::from_secs(60))
        .unwrap();
    let stored: Vec<String> = knowledge
        .conn()
        .prepare("SELECT token_hash FROM session")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(!stored.contains(&token));
}

// History

#[test]
fn revisions_list_the_history_newest_first_with_authors() {
    let knowledge = knowledge();
    let claude = Author::Agent("claude-code".into());
    let owner = Author::Owner("user-1".into());
    knowledge
        .write_page("Proxy", "Traefik", None, PageChanges::default(), &claude)
        .unwrap();
    knowledge
        .write_page(
            "Proxy",
            "Caddy, because of automatic HTTPS",
            Some(1),
            PageChanges {
                new_title: Some("Reverse proxy".into()),
                page_type: Some(Some("decision".into())),
                ..Default::default()
            },
            &owner,
        )
        .unwrap();

    let history = knowledge.revisions("proxy").unwrap().unwrap();
    let summary: Vec<_> = history
        .iter()
        .map(|r| (r.number, r.title.as_str(), r.author.clone(), r.body_bytes))
        .collect();
    assert_eq!(
        summary,
        [
            (2, "Reverse proxy", Some(owner), 33),
            (1, "Proxy", Some(claude), 7)
        ]
    );
    assert_eq!(history[0].page_type.as_deref(), Some("decision"));
    assert_eq!(knowledge.revisions("Nothing").unwrap(), None);
}

// Statistics

/// Moves a page's (by title) or note's (by ID) creation to another time.
fn created_at(knowledge: &Knowledge, table: &str, key: &str, at: &str) {
    let column = if table == "page" { "title" } else { "id" };
    knowledge
        .conn()
        .execute(
            &format!("UPDATE {table} SET created_at = ?1 WHERE {column} = ?2"),
            (at, key),
        )
        .unwrap();
}

#[test]
fn totals_count_pages_stubs_links_notes_and_revisions() {
    let knowledge = knowledge();
    assert_eq!(knowledge.totals().unwrap(), Totals::default());

    create(&knowledge, "Rust", "[[Rust]] [[Cargo]]");
    let mnemograph = create(&knowledge, "mnemograph", "[[Rust]]");
    update(
        &knowledge,
        "mnemograph",
        "[[Rust]] [[Tantivy]]",
        mnemograph.revision,
    )
    .unwrap();
    knowledge
        .add_note("[[Rust]] and [[Caddy]]", &author())
        .unwrap();

    assert_eq!(
        knowledge.totals().unwrap(),
        Totals {
            pages: 2,
            // Cargo and Tantivy from pages, Caddy from the note.
            stubs: 3,
            // Rust's link to itself does not count; the note's two do.
            links: 5,
            notes: 1,
            revisions: 3,
        }
    );
}

#[test]
fn weekly_growth_counts_pages_and_notes_up_to_each_week() {
    let knowledge = knowledge();
    assert!(
        knowledge
            .weekly_growth_until("2026-10-29")
            .unwrap()
            .is_empty()
    );

    create(&knowledge, "A", "");
    create(&knowledge, "B", "");
    let note = knowledge.add_note("first", &author()).unwrap();
    // Thursday 2026-10-08 and Sunday 2026-10-11 are in the week of Monday 2026-10-05; Monday
    // 2026-10-19 starts its own week. The week of 10-12 has nothing new.
    created_at(&knowledge, "page", "A", "2026-10-08T10:00:00.000Z");
    created_at(
        &knowledge,
        "note",
        &note.id.to_string(),
        "2026-10-11T23:59:00.000Z",
    );
    created_at(&knowledge, "page", "B", "2026-10-19T00:00:00.000Z");

    let weeks = knowledge.weekly_growth_until("2026-10-29").unwrap();
    let summary: Vec<_> = weeks
        .iter()
        .map(|w| (w.start.as_str(), w.pages, w.notes))
        .collect();
    assert_eq!(
        summary,
        [
            ("2026-10-05", 1, 1),
            ("2026-10-12", 1, 1),
            ("2026-10-19", 2, 1),
            ("2026-10-26", 2, 1),
        ]
    );
}

#[test]
fn a_wiki_only_days_old_has_one_week() {
    let knowledge = knowledge();
    create(&knowledge, "A", "");
    let weeks = knowledge.weekly_growth().unwrap();
    assert_eq!(weeks.len(), 1);
    assert_eq!((weeks[0].pages, weeks[0].notes), (1, 0));
}

#[test]
fn most_linked_and_orphaned_pages() {
    let knowledge = knowledge();
    create(&knowledge, "Hub", "[[A]] [[B]] [[Stub]]");
    create(&knowledge, "A", "[[B]]");
    create(&knowledge, "B", "");
    create(&knowledge, "zeta", "");
    create(&knowledge, "Alone", "[[Alone]]");
    knowledge.add_note("[[zeta]]", &author()).unwrap();

    let most: Vec<_> = knowledge
        .most_linked_pages(2)
        .unwrap()
        .into_iter()
        .map(|n| (n.title, n.degree))
        .collect();
    assert_eq!(most, [("Hub".to_owned(), 3), ("A".to_owned(), 2)]);
    // A link to itself and a note's link do not connect a page.
    let orphans: Vec<_> = knowledge
        .orphaned_pages()
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    assert_eq!(orphans, ["Alone", "zeta"]);
}

#[test]
fn activity_per_author_counts_revisions_and_notes() {
    let knowledge = knowledge();
    assert!(knowledge.author_activity().unwrap().is_empty());
    assert_eq!(
        knowledge.weekly_activity_until("2026-10-29").unwrap(),
        WeeklyActivity::default()
    );

    let claude = Author::Agent("claude-code".into());
    let vibe = Author::Agent("vibe".into());
    let page = knowledge
        .write_page("A", "one", None, PageChanges::default(), &claude)
        .unwrap();
    knowledge
        .write_page(
            "A",
            "two",
            Some(page.revision),
            PageChanges::default(),
            &claude,
        )
        .unwrap();
    knowledge.add_note("from claude", &claude).unwrap();
    knowledge.add_note("from vibe", &vibe).unwrap();
    {
        let conn = knowledge.conn();
        // Claude's first revision two weeks earlier; one row from before authors were recorded.
        conn.execute(
            "UPDATE revision SET created_at = '2026-10-14T09:00:00.000Z' WHERE number = 1",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE revision SET created_at = '2026-10-27T09:00:00.000Z' WHERE number = 2",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE note SET created_at = '2026-10-28T09:00:00.000Z' WHERE body = 'from claude'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE note SET created_at = '2026-10-21T09:00:00.000Z', author = NULL \
             WHERE body = 'from vibe'",
            [],
        )
        .unwrap();
    }
    knowledge.add_note("vibe again", &vibe).unwrap();
    knowledge
        .conn()
        .execute(
            "UPDATE note SET created_at = '2026-10-22T09:00:00.000Z' WHERE body = 'vibe again'",
            [],
        )
        .unwrap();

    let activity: Vec<_> = knowledge
        .author_activity()
        .unwrap()
        .into_iter()
        .map(|a| (a.author, a.revisions, a.notes, a.first, a.last))
        .collect();
    assert_eq!(
        activity,
        [
            (
                Some(claude.clone()),
                2,
                1,
                "2026-10-14T09:00:00.000Z".to_owned(),
                "2026-10-28T09:00:00.000Z".to_owned()
            ),
            (
                Some(vibe.clone()),
                0,
                1,
                "2026-10-22T09:00:00.000Z".to_owned(),
                "2026-10-22T09:00:00.000Z".to_owned()
            ),
            (
                None,
                0,
                1,
                "2026-10-21T09:00:00.000Z".to_owned(),
                "2026-10-21T09:00:00.000Z".to_owned()
            ),
        ]
    );

    let weekly = knowledge.weekly_activity_until("2026-11-04").unwrap();
    assert_eq!(
        weekly.weeks,
        ["2026-10-12", "2026-10-19", "2026-10-26", "2026-11-02"]
    );
    assert_eq!(
        weekly.authors,
        [
            (Some(claude), vec![1, 0, 2, 0]),
            (Some(vibe), vec![0, 1, 0, 0]),
            (None, vec![0, 1, 0, 0]),
        ]
    );
}

// Access log

#[test]
fn the_access_log_counts_reads_searches_and_gaps_per_agent() {
    let knowledge = knowledge();
    let claude = Author::Agent("claude-code".into());
    let vibe = Author::Agent("vibe".into());
    knowledge
        .write_page(
            "Rust",
            "",
            None,
            PageChanges {
                aliases: Some(vec!["rustlang".into()]),
                ..Default::default()
            },
            &claude,
        )
        .unwrap();
    create(&knowledge, "Gone", "");
    let read = Access::ReadPage;
    knowledge.record_access(&claude, read, "Rust", 1).unwrap();
    knowledge
        .record_access(&claude, read, "rustlang", 1)
        .unwrap();
    knowledge.record_access(&vibe, read, "RUST", 1).unwrap();
    knowledge.record_access(&vibe, read, "Gone", 1).unwrap();
    knowledge.record_access(&vibe, read, "Missing", 0).unwrap();
    knowledge
        .record_access(&claude, Access::Search, "Datenbank", 3)
        .unwrap();
    knowledge
        .record_access(&claude, Access::Search, "Garage  backup", 0)
        .unwrap();
    knowledge
        .record_access(&vibe, Access::Search, "garage backup", 0)
        .unwrap();
    knowledge
        .record_access(&vibe, Access::Search, "Hetzner", 0)
        .unwrap();
    knowledge
        .record_access(&claude, Access::Related, "Rust", 2)
        .unwrap();
    knowledge.delete_page("Gone").unwrap();

    let agents: Vec<_> = knowledge
        .agent_access()
        .unwrap()
        .into_iter()
        .map(|a| (a.author.unwrap(), a.reads, a.searches, a.related))
        .collect();
    // Both did five things; claude's last came later.
    assert_eq!(agents, [(claude.clone(), 2, 2, 1), (vibe.clone(), 3, 2, 0)]);

    // Reads under an alias or another spelling count for the page; deleted pages and failed
    // reads are left out.
    let most_read = knowledge.most_read_pages(10).unwrap();
    assert_eq!(
        most_read,
        [PageReads {
            title: "Rust".into(),
            reads: 3
        }]
    );

    let gaps: Vec<_> = knowledge
        .knowledge_gaps(10)
        .unwrap()
        .into_iter()
        .map(|g| (g.terms, g.times, g.author.unwrap()))
        .collect();
    assert_eq!(
        gaps,
        [
            ("Hetzner".to_owned(), 1, vibe.clone()),
            ("garage backup".to_owned(), 2, vibe.clone()),
        ]
    );

    let weekly = knowledge.weekly_access().unwrap();
    assert_eq!(weekly.weeks.len(), 1);
    assert_eq!(
        weekly.authors,
        [(Some(claude), vec![5]), (Some(vibe), vec![5])]
    );
}

#[test]
fn old_access_log_entries_are_pruned() {
    let knowledge = knowledge();
    let agent = author();
    knowledge
        .record_access(&agent, Access::Search, "old", 0)
        .unwrap();
    knowledge
        .record_access(&agent, Access::Search, "new", 0)
        .unwrap();
    knowledge
        .conn()
        .execute(
            "UPDATE access SET at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-91 days') \
             WHERE target = 'old'",
            [],
        )
        .unwrap();
    assert_eq!(knowledge.prune_access_log(90).unwrap(), 1);
    assert_eq!(knowledge.prune_access_log(90).unwrap(), 0);
    let gaps = knowledge.knowledge_gaps(10).unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].terms, "new");
}
