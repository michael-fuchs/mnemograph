# Embedded storage options (research, 2026-10-18)

Facts gathered to choose the embedded storage engine for mnemograph. These are facts, not a recommendation.
They complement `2026-10-initial-stack-facts.md`, which covers LadybugDB (`lbug`) in depth.

What the storage has to do: a few thousand to tens of thousands of Markdown pages, a revision of every page, short notes, page links (backlinks and neighborhood queries up to about 3 hops), full-text search with ranking and German + English stemming, and optional vector search later. It runs as a single process with a single owner on a 2 vCPU / 4 GB VPS, embedded in-process, and the data must be exportable.

Sources are primary (crates.io API, GitHub API/repos, official docs, source code) unless marked otherwise.
Items marked **UNVERIFIED** could not be confirmed from a primary source.
Star counts and dates are as of 2026-10-18.

---

## 1. Turso (Rust rewrite of SQLite) and libSQL

### Status and versions

| Fact | Value | Source |
| --- | --- | --- |
| Latest `turso` / `turso_core` crate | **0.8.3** stable (2026-10-12); 0.9.0-pre.1 (2026-10-15). There are 2–3 releases a week, with `-pre` tags in between. | https://crates.io/crates/turso, https://github.com/tursodatabase/turso/releases |
| License | MIT | https://github.com/tursodatabase/turso/blob/main/LICENSE.md |
| Repo | https://github.com/tursodatabase/turso: 24,814 stars, about 347 contributors, last push 2026-10-18 | GitHub API |
| Backing | Turso (the company behind Turso Cloud and libSQL) | README |
| Status (self-declared) | "It runs in production today at multiple organizations" (Turso Cloud, Kin AI, Spice.ai). It is **pre-1.0**: "some features are explicitly marked experimental… until we declare 1.0… keep independent backups." The libSQL README still calls Turso "currently in beta". | https://github.com/tursodatabase/turso#faq, https://github.com/tursodatabase/libsql |
| Testing | Deterministic simulation testing and Antithesis; differential testing against SQLite 3.50.4 | README, COMPAT.md |
| Open issues | 1,023 open issues; 48 open issues match "corrupt". Recent data-loss bugs were fixed recently, for example #10032, "A -wal with a bad header checksum … main file is truncated at close (all committed rows lost)", closed 2026-10-16. | GitHub search API |
| Pure Rust | The engine is Rust. Default features pull in the C crates `libmimalloc-sys` and `zstd-sys`. Build: 134 s, 23 MB binary (see §6). | `bindings/rust/Cargo.toml` |

### File format and SQLite compatibility

- COMPAT.md says: "✅ SQLite file format is fully supported". It also lists these guarantees: "You should always be able to go back to SQLite", "access a database created with SQLite in Turso", and "You need to opt in to any incompatible Turso feature". Source: https://github.com/tursodatabase/turso/blob/main/COMPAT.md
- Only WAL journaling is supported. Rollback-journal modes are "Not Needed", and legacy databases are converted to WAL when they are opened.
- Text must be valid UTF-8. Invalid bytes become U+FFFD when converted to text, which differs from SQLite.
- "We don't support mixed SQLite and Turso in multi-process scenarios." The manual says there is no multi-process access. Experimental multi-process WAL (`.tshm` sidecar) exists behind a flag.
- The FTS index is stored as Turso-specific B-tree rows (a "segment registry"; see below). **UNVERIFIED** whether stock SQLite can open a file that contains a Turso FTS index. It is an opt-in incompatible feature by Turso's own definition.

### Transactions and concurrency

- Default mode is WAL: many readers and a single writer, like SQLite.
- MVCC is a journal mode (`PRAGMA journal_mode = mvcc`) and enables `BEGIN CONCURRENT`. That gives snapshot isolation with row-level write-write conflict detection; conflicting commits get `SQLITE_BUSY` and must be retried. The manual says MVCC is "**not production ready so do not use it for critical data right now**". Source: https://github.com/tursodatabase/turso/blob/main/docs/manual.md#journal-mode
- Each connection has one transaction. To get concurrency you use multiple connections.

### SQL feature gaps that matter for mnemograph

From COMPAT.md (main branch, 2026-10-18) and the issue tracker:

- **Recursive CTEs**: COMPAT.md still says "WITH RECURSIVE not yet supported", but this is stale. PR #8050 "WITH RECURSIVE" was merged on 2026-07-30. Correctness bugs are still open:
  - **#8726 (open)**: "A recursive CTE whose arm joins two tables stops after one level… Rows go missing with no error". A graph walk that joins the CTE to an edge table and a node table has exactly this shape.
  - #8754 (open): a recursive CTE join drops affinity.
  - #8976 (open): recursive CTEs are slower than in sqlite3.
- Triggers: `CREATE TRIGGER` is ✅. `INSTEAD OF` triggers on views are ❌. `PRAGMA recursive_triggers` is ❌. The manual's "Limitations" section still says "No triggers / No views / No savepoints / No vacuum". It contradicts COMPAT.md and appears to be outdated.
- Window functions are partial: `lag`/`lead`/`ntile` are missing, and so are custom frames.
- Generated columns and `WITHOUT ROWID` are experimental and behind flags.
- `VACUUM INTO` is supported. In-place `VACUUM` is experimental.
- **Backup API** (`sqlite3_backup_*`) is ❌ (stub). Serialization is ❌. Loadable extensions are ❌. Virtual-table C API (`sqlite3_create_module`) is ❌, so FTS5, sqlite-vec and similar cannot be plugged in.
- **SQLite FTS3/FTS4/FTS5 are ❌**. You have to use Turso's own FTS.

### Full-text search (Tantivy-based)

- `CREATE INDEX … USING fts (title, body) WITH (tokenizer=…, weights='title=2.0,body=1.0')`, with `fts_match()`, `fts_score()` (BM25) and `fts_highlight()`, and the Tantivy query syntax (boolean, phrase, prefix, field, boost). There is no `snippet()`. "The index automatically tracks inserts, updates, and deletes." There is no read-your-writes inside a transaction: FTS changes only become visible after COMMIT. Source: manual §Full-Text Search
- The manual marks it **Experimental**. It is behind the `fts` cargo feature, which is on by default in the `turso` Rust crate. Open issue #7800 (2026-07-08) says: "Custom index modules (Tantivy FTS, vector) are not supported in MVCC mode". This conflicts with the MVCC design notes in `fts/mod.rs`, so it may be stale; **UNVERIFIED**.
- **Tokenizers**: `default`, `raw`, `simple`, `whitespace` and `ngram`, and nothing else. The code hard-codes `SUPPORTED_TOKENIZERS` and rejects other names. **There is no stemming for any language**, so German or English stemming is not available. Source: https://github.com/tursodatabase/turso/blob/main/core/index_method/fts/mod.rs
- Storage: Tantivy segments are stored as immutable segments plus a "segment registry" of MVCC-versioned rows in the backing B-tree, so the index lives in the database file. Turso pins `tantivy = 0.26.2`. Source: same file, `core/Cargo.toml`

### Vector

- Vector functions are compatible with libSQL: `vector32`/`vector64`/sparse/`vector8`/binary types, and `vector_distance_cos`/`_l2`/`_jaccard`, and so on. This gives exact (brute-force) search.
- An approximate vector index is still "**on our current roadmap**". The experimental "Index Method" framework (`--experimental-index-method`) is the mechanism for it. Source: README, manual §Vector search, §Index Method

### Backup and export while running

- There is no online backup API. Options are `VACUUM INTO` (supported), the `tursodb` shell `.dump`, and the WAL frame APIs (`libsql_wal_get_frame`, …). Because the format is SQLite-compatible, stock `sqlite3` can read a copied, checkpointed file. That only holds if no Turso-only features such as FTS or MVCC are used. **UNVERIFIED**.

### libSQL (for contrast)

- `libsql` crate **0.10.0** (2026-10-13). It is MIT-licensed and a C fork of SQLite (17,280 stars). The README says: "libSQL is actively maintained, but new features are being developed in Turso. If you're starting a new project, you probably want to look into Turso." Source: https://github.com/tursodatabase/libsql
- Being an SQLite fork, it inherits FTS5, recursive CTEs, triggers and the backup API. It adds native vector indexes (see turso.tech/vector). The C build goes through `libsql-sys`. The default crate features pull in replication, remote, sync and tls (tonic, hyper), so an embedded-only build needs `default-features = false, features = ["core"]`.
- Commit activity is low: the most recent commits are 2026-10-13 (the 0.10.0 release) and before that 2026-08-23.

---

## 2. SurrealDB embedded

### Status and versions

| Fact | Value | Source |
| --- | --- | --- |
| Latest `surrealdb` crate | **3.3.4** (2026-10-13). The 3.x line: 3.0 (2026-02-17), 3.1 (2026-05-26), 3.2 (2026-07-02), 3.3 (2026-09-24). 2.x is still patched (2.7 on 2026-09-23). | crates.io API |
| Major-version history | 1.0 (2023-09-13), 2.0 (2024-09-17), 3.0 (2026-02-17). There have been **3 majors in about 2.5 years**. | crates.io API |
| Repo | https://github.com/surrealdb/surrealdb: 33,201 stars, last push 2026-10-08, 583 open issues | GitHub API |
| Backing | SurrealDB Ltd (VC-funded company; funding details **UNVERIFIED**) | LICENSE |
| Embedded engines | Cargo features `kv-mem`, `kv-surrealkv` (pure Rust) and `kv-rocksdb` (C++ via `surrealdb-rocksdb` 0.24.0-surreal.4). Also `kv-indxdb` (WASM) and `kv-tikv` (remote). The default features are `protocol-ws` and `rustls` (a client), so for embedded use you disable the defaults. | https://github.com/surrealdb/surrealdb/blob/main/surrealdb/Cargo.toml |
| SurrealKV | `surrealkv` **1.0.0** (2026-10-13), Apache-2.0, pure-Rust LSM tree. SurrealDB main still pins `surrealkv = "0.21.2"`. The SurrealKV README warns: "The on-disk formats are not frozen yet, so do not rely on them for data that must survive an upgrade without a backup." Opening a v0.20/0.21 store with 1.0 converts it one way, after which "a database that this version has opened can no longer be opened by an earlier release". | https://github.com/surrealdb/surrealkv (README), surrealdb `Cargo.toml` |

### License (important)

- The `surrealdb` crate (the SDK, including the embedded engine) uses `license-file`, and crates.io reports it as "non-standard". It points to the repo's **Business Source License 1.1**. Source: https://github.com/surrealdb/surrealdb/blob/main/LICENSE
  - Licensed Work: "SurrealDB 3.0". Licensor: SurrealDB Ltd.
  - **Additional Use Grant**: "You may make use of the Licensed Work, provided that you may not use the Licensed Work as a Database Service." A "Database Service" is "any product, service, platform, or commercial offering in which the Licensed Work is used to provide database functionality to third parties, other than your direct employees or contractors…, that enables third parties to create, manage, or control schemas or tables."
  - **Change Date: 2030-01-01**, or four years after first public distribution of a version, whichever is first. The Change License is **Apache-2.0**.
  - "The Business Source License … is not an Open Source license."
- What this means for mnemograph (my reading, not legal advice): a self-hosted wiki that uses SurrealDB internally, where users can't create or control schemas or tables, looks covered by the grant. There are two consequences to note:
  1. Mnemograph's binary would contain BSL-licensed code, so redistributing it means passing the BSL terms on to anyone who runs it. Mnemograph could not be offered as "pure" OSI open source until each SurrealDB version converts.
  2. If a feature ever let users or agents define their own tables or schemas for third parties, that could fall under "Database Service".
- `surrealkv` itself (the KV layer only) is Apache-2.0.

### Graph features

- `RELATE a->edge->b` creates edges as records, and edges can carry properties. Traversal uses `->edge->table`, `<-` and `<->`.
- Recursive traversal syntax (since **2.1.0**, 2024-11-21): `@.{1..3}(->links_to->page)`, with `+path`, `+collect` (the deduplicated set of visited nodes) and `+shortest=record:id`. "A path without an upper bound stops at a depth of 256." Sources: https://surrealdb.com/docs/learn/data-models/graph/recursive-traversals.md, https://surrealdb.com/blog/surrealdb-2-1-0-is-live.md

### Full-text search

- `DEFINE ANALYZER` takes tokenizers `blank`, `camel`, `class`, `punct` and `segment` (CJK). Filters include `ascii`, `lowercase`, `edgengram`, `ngram`, `mapper` and **`snowball(language)`**, whose languages include **German and English** (17 languages in total). Source: https://surrealdb.com/docs/reference/query-language/statements/define/analyzer.md
- `DEFINE INDEX … FULLTEXT ANALYZER x BM25 HIGHLIGHTS` (3.x renamed `SEARCH ANALYZER` to `FULLTEXT ANALYZER`) works with `search::score()` and `search::highlight()`. Indexes can be built `CONCURRENTLY`. Source: https://surrealdb.com/docs/reference/query-language/statements/define/indexes.md

### Vector

- An HNSW index (defaults M=12, EFC=150; the vector cache is bounded, but "the in-memory graph itself must be sized for separately"). There is also a DiskANN index for graphs that don't fit in RAM. 3.x replaced MTREE with HNSW. Source: same indexes page.

### On-disk format and upgrades

- "Across **major versions**, on-disk formats may change." The **2.x → 3.x upgrade cannot be done in place**: you export with `surreal v2 export --v3` (needs 3.0.3+) or with the Studio "V3 Compatible Export" (2.6.0+), then import. Breaking query-language changes include removed futures, 40+ renamed functions, `array::range` semantics, required `LET`, removed fuzzy-match operators, `SEARCH` → `FULLTEXT`, MTREE → HNSW, and `?` → `.?`. 1.x → 2.x also had its own guide. Sources: https://surrealdb.com/docs/build/migrating/from-old-surrealdb-versions/2x-to-3x.md, https://surrealdb.com/docs/manage/self-hosted/upgrades-and-patching.md
- From **3.3.0**, a datastore records the version that last opened it and runs pending data migrations on startup. A build that lacks a recorded migration refuses to start, so going back to an older version means restoring a backup. (Same source.)

### Backup and export while running

- The Rust SDK has `db.export("backup.sql")` / `db.export(())`, which writes a SurrealQL dump to a file or a byte stream, plus `db.import(...)`. It is a logical export from the running process. Source: `surrealdb/src/method/mod.rs`
- Opening the store from a second process while the app holds it: **UNVERIFIED**. For RocksDB this is normally blocked by its LOCK file.

### Compile time, binary size and memory

- Measured: a **398 s clean release build (16 threads), a 108 MB stripped binary and 511 dependencies** for `kv-surrealkv` alone (see §6). With `kv-rocksdb`, a C++ RocksDB build comes on top of that (not measured).
- Memory use of embedded mode on a 4 GB VPS: there is no official figure (**UNVERIFIED**). The startup RSS measured for a trivial embedded program is in §6.
- Maturity of embedded mode: it is documented and supported, but upgrade churn (above) applies to embedded use too.

---

## 3. Baseline: rusqlite (bundled SQLite) with FTS5 and recursive CTEs

| Fact | Value | Source |
| --- | --- | --- |
| `rusqlite` | **0.40.2** (2026-08-08), MIT, 4,436 stars, last push 2026-10-18 | crates.io, GitHub API |
| `libsqlite3-sys` | 0.38.2. The bundled SQLite in the repo's `sqlite3.h` is **3.54.0** (main branch; the exact version bundled in 0.38.2 is **UNVERIFIED**). | https://github.com/rusqlite/rusqlite/tree/master/libsqlite3-sys |
| FTS5 in the bundled build | Yes: `build.rs` compiles with `-DSQLITE_ENABLE_FTS5`, `FTS3`, `JSON1`, `RTREE`, `STAT4` and `LOAD_EXTENSION` | `libsqlite3-sys/build.rs` |
| Backup while running | `rusqlite::backup` (online backup API, `backup` feature) and `VACUUM INTO`. WAL mode allows readers during a backup. | rusqlite features list |
| Recursive CTEs, triggers | Fully supported (core SQLite) | sqlite.org |
| C dependency | Yes, `sqlite3.c` compiled by `cc`. That is a single C file with no cmake. | build.rs |

FTS5 tokenizers (https://sqlite.org/fts5.html#tokenizers):

- `unicode61` is the default: Unicode case folding, with optional diacritics removal.
- `ascii`.
- `porter` is a wrapper that applies the **English-only** Porter stemmer. The docs say: "The porter stemmer algorithm is designed for use with English language terms only".
- `trigram` supports substring and LIKE queries.
- Ranking: built-in `bm25()` with per-column weights, plus `highlight()` and `snippet()`.

Options for German stemming with FTS5:

1. Write a **custom FTS5 tokenizer** through `fts5_api` (the `xCreateTokenizer` C API) that wraps a Rust Snowball stemmer, for example `frostem` (what Tantivy now uses) or `rust-stemmers`. rusqlite has no safe API for this; issue #689 "Custom FTS5 tokenizer support" has been open since 2020. A small third-party crate exists: `rusqlite-ext` 0.39.0 (lukingcathy), which builds FTS5 tokenizers for rusqlite. It is little-used (871 downloads) and its maturity is **UNVERIFIED**.
2. **Pre-stem in the application**: store a normalized/stemmed shadow text in an external-content FTS5 table, and stem the query terms the same way.
3. Keep FTS outside SQLite in **Tantivy** (§4).

Vector search with SQLite: `sqlite-vec` 0.1.10-alpha.4 (2026-05-18; MIT/Apache-2.0; 8,194 stars). It is still alpha and there has been no push since 2026-05-18. Brute-force search with plain BLOB columns and app-side distance is also feasible at mnemograph's scale.

---

## 4. Tantivy (search library)

| Fact | Value | Source |
| --- | --- | --- |
| Latest | **0.27.0** (2026-10-17). Earlier: 0.26.2 (2026-09-08), 0.26.0 (2026-03-31), 0.25.0 (2025-08-20). Still pre-1.0, with breaking API changes per minor (see the CHANGELOG). | crates.io, https://github.com/quickwit-oss/tantivy/blob/main/CHANGELOG.md |
| License | MIT. 16,233 stars, last push 2026-10-17, MSRV 1.88 | GitHub API, Cargo.toml |
| German stemming | Yes. The default `stemmer` feature uses `frostem` (it replaced the unmaintained `rust-stemmers` in 0.27, PR #3028). `tokenizer::Language::German` maps to the Snowball German algorithm. | `Cargo.toml`, `src/tokenizer/stemmer.rs` |
| Ranking | BM25, phrase and fuzzy queries, highlighting (snippets) | docs.rs/tantivy |
| Pure Rust | Yes (`zstd` is optional) | Cargo.toml |
| Ownership | The quickwit-oss org; contributors include `fulmicoton-dd`. Quickwit's acquisition by Datadog is **UNVERIFIED** here (secondary sources only). | GitHub |
| Index format | Tantivy indexes are versioned per release. Tantivy is a derived index you can rebuild from the primary store, so format churn costs a reindex, not data loss. |  |

---

## 5. Other Rust-native options (brief)

| Project | Latest / date | License | Activity & maturity | Notes | Source |
| --- | --- | --- | --- | --- | --- |
| **HelixDB** (`helix-db`) | 3.0.0 (2026-08-04); 2.0 (2026-05), 1.0 (2025-07) | Apache-2.0 | 6,211 stars, active (push 2026-10-18). Top contributor `xav-db` has 2,429 commits; the next has 421. | It positions itself as a graph + vector + BM25 database "built in Rust on object storage". Embedded mode is `helix-db` with the `embedded` feature, backed by memory, disk or S3. **3 majors in about 15 months**. Stemming support is **UNVERIFIED**. The project is primarily server- and cloud-oriented. | https://github.com/HelixDB/helix-db, https://docs.helix-db.com/database/helix-db/start-here/local-development/embedded-database |
| **Grafeo** (`grafeo`) | 0.5.44 (2026-10-04), 0.6.0-beta.1 (2026-10-11); first release 2026-01-29 | Apache-2.0 | 814 stars, about 50 releases in 9 months. **One dominant author** (`StevenBtw`, 1,423 commits; the next has 53). MSRV 1.99. | Embedded LPG + RDF, with GQL, Cypher, Gremlin, SPARQL and SQL/PGQ. It has HNSW, BM25 ("Unicode tokenizer and stop word removal"; stemming **UNVERIFIED**), MVCC, a WAL and `wal_checkpoint`. It is very young. | https://github.com/GrafeoDB/grafeo |
| **IndraDB** | 5.0.0 (2025-08-16); 4.0.0 was 2023 | MPL-2.0 | 2,467 stars. Last push 2025-08-16, so **dormant** | A graph database (property graph), with no FTS or vector search | https://github.com/indradb/indradb |
| **CozoDB** (`cozo`) | 0.7.6 (2023-12-11) | MPL-2.0 | 4,133 stars. Last commit 2024-12-04, so **effectively abandoned** | Datalog with HNSW and FTS | https://github.com/cozodb/cozo |
| **Oxigraph** | 0.5.11 (2026-09-02) | MIT OR Apache-2.0 | 1,997 stars, active | An RDF/SPARQL store. Persistent storage **requires RocksDB (C++)** through `oxrocksdb-sys`; without it, storage is in-memory only. It has no FTS. | https://github.com/oxigraph/oxigraph |
| **native_db** | 0.8.2 (2025-07-08) | MIT | 718 stars, push 2026-10-16, but no release for 15 months | A typed object store on top of redb, with secondary indexes. No query language, FTS or graph. | https://github.com/vincent-herlemont/native_db |
| **redb** | 4.4.0 (2026-10-13) | MIT OR Apache-2.0 | 4,853 stars, very active | A pure-Rust B-tree KV store with ACID, MVCC and savepoints. The README says: "Stable and maintained. The file format is stable, and a reasonable effort will be made to provide an upgrade path". Majors have still changed the file format: 3.0 (2025-08-09) "Removes support for file format v2", with `Database::upgrade()` in 2.6. | https://github.com/cberner/redb |
| **fjall** | 3.1.12 (2026-10-03) | MIT OR Apache-2.0 | 2,385 stars, active | A pure-Rust LSM KV store with MVCC snapshots. It promises a "Stable disk format", and "Future breaking changes will result in a major version bump and a migration path." | https://github.com/fjall-rs/fjall |
| **sled** | 1.0.0-alpha.124 (2024-10-11) | MIT OR Apache-2.0 | Stalled in alpha | Listed for completeness | crates.io |

Roll-your-own option (`redb`/`fjall` + `tantivy`): everything is pure Rust. In exchange you have to write the graph layer (adjacency tables for links and backlinks, BFS in application code), the revision model, transactions across the KV store and the search index (Tantivy commits are separate from KV commits), and the export format yourself.

---

## 6. Local build measurements

Measured on 2026-10-18 on the workspace machine: 16 threads, 62 GB RAM, rustc 1.99.0. Each crate is a minimal `main.rs` that opens a database and runs one statement. `cargo build --release` used a cold, separate target directory per crate, with `strip = true`. Dependencies were fetched beforehand, so download time is excluded. On the 2 vCPU target VPS, build times would be several times longer. They are only meant to compare the candidates relative to each other.

| Crate (config) | Clean release build | Stripped binary | Unique deps (`cargo tree -e normal`) | Max RSS for open + one trivial query | C/C++ `-sys` deps seen |
| --- | --- | --- | --- | --- | --- |
| `rusqlite` 0.40.2 `bundled` | 46 s | 2.4 MB | 10 | 4.8 MB | sqlite3.c (cc) |
| `tantivy` 0.27.0 (defaults) | 49 s | 0.6 MB | 132 | n/a | none |
| `turso` 0.8.3 (defaults: `fts`, `mimalloc`) + tokio | 134 s | 23 MB | 316 | 25 MB | `libmimalloc-sys`, `zstd-sys` |
| `surrealdb` 3.3.4 `default-features=false, kv-surrealkv` + tokio | **398 s** | **108 MB** | 511 | 59 MB | `aws-lc-sys` (C, also needs cmake/clang in some setups) |

Caveats: the RSS figures come from an empty database and say nothing about steady-state memory under load. The binary sizes include only the database, not axum, rmcp and so on. The "Pure Rust" column below means "no C++ storage engine". Even the SurrealKV build of SurrealDB pulls in `aws-lc-sys` (C), and Turso pulls in mimalloc and zstd (C).

---

## 7. Hard constraints: vector search and mixed German + English FTS

Two requirements were raised to hard constraints after the first pass:

- **(V)** The engine must support vector search, without a second database and without relying on a pre-1.0 or abandoned extension. It does not have to be in the first version.
- **(F)** Full-text search over mixed German and English content must stem both languages.

### 7.1 Vector options: concrete maturity

| Option | ANN algorithm | Status / maturity evidence | Meets (V)? | Source |
| --- | --- | --- | --- | --- |
| **sqlite-vec** (for rusqlite) | Stable releases offer brute-force KNN only (`vec0`). IVF/DiskANN-style ANN exists only in the **0.1.10-alpha** line, which has "IVF shadow tables". | **Pre-v1**: the README says "`sqlite-vec` is a pre-v1, so expect breaking changes!" Stable releases are 0.1.6 (2024-11-20), then nothing for 16 months, then 0.1.7–0.1.9 (2026-03). The latest is 0.1.10-alpha.4 (2026-05-18), and there have been **no commits since 2026-05-18**. 159 open issues, one maintainer (asg017), sponsored by Mozilla Builders. | **No**: pre-1.0 and stalled for 5 months | https://github.com/asg017/sqlite-vec, crates.io |
| **Brute force in SQLite** (BLOB column + Rust distance function as an `rusqlite` scalar function or in app code) | None; exact search | There is nothing to mature. At 10k–100k chunks × 768–1024 dims (f32), a scan reads about 30–400 MB, which takes tens to hundreds of ms on 2 vCPU (estimate, **UNVERIFIED** by benchmark). | Yes, if you accept the scale limit; no external extension needed | n/a |
| **libSQL native vector** | **DiskANN** (`CREATE INDEX … (libsql_vector_idx(col))`, `vector_top_k()`), cosine/L2, up to 65,536 dims | In-tree C in `libsql-sqlite3/src/vectordiskann.c` and related files, so it works embedded. But libSQL as a whole is in maintenance mode ("new features are being developed in Turso"). Recent open bug: #2289 (2026-10-12) "Unqualified DELETE leaves libsql_vector_idx shadow rows and breaks the next insert". 13 open vector issues. | Partly: works today, but on a de-prioritized codebase | https://docs.turso.tech/features/ai-and-embeddings, https://github.com/tursodatabase/libsql |
| **Turso native vector** | **None yet.** Exact search only through `vector_distance_*`. The ANN index is "on our current roadmap". An experimental `index_method` framework exists (a toy sparse IVF example). | Open issue #7800: "Custom index modules (Tantivy FTS, vector) are not supported in MVCC mode". | **No** ANN today; exact search only | Turso README, manual, GitHub issues |
| **SurrealDB HNSW / DiskANN** | HNSW (M=12, EFC=150 defaults), plus DiskANN for larger-than-RAM data. Both are built in, with no extension. | GA in 3.x, but **17 open HNSW issues**, including #7544 (2026-09-27) "HNSW: a KNN query returns wrong neighbours and leave the index permanently broken", #7542/#7543 (wrong results while an index builds `CONCURRENTLY`), and #7575 (DiskANN import drops batches on SurrealKV, 3.3.0). | Yes, feature-wise; correctness bugs are open | https://surrealdb.com/docs/reference/query-language/statements/define/indexes.md, GitHub issues |
| **LadybugDB `vector` extension** | HNSW (two-layer), with cosine/L2/l2sq/dot. Index only on node-table properties. | A runtime-loaded shared extension (see the other research doc). Whether the index is maintained on insert/update/delete is not documented (**UNVERIFIED**). Bus factor ≈1. | Built into the engine's ecosystem, but pre-1.0 (0.21.x) | https://docs.ladybugdb.com/extensions/vector |
| **Tantivy-adjacent** (paired with any KV/SQL store) | Tantivy 0.27 has **no** vector index. Separate crates: `hnsw_rs` 0.3.5 (2026-10-09, pure Rust, MIT/Apache, 1.18 M downloads), `arroy` 0.8.0 (2026-08-12, Meilisearch, LMDB-backed, MIT), `usearch` 2.26.4 (2026-10-05, C++ core, Apache-2.0). `instant-distance` (last release 2023-06) is dormant. | All are libraries, not databases. You would persist and rebuild the index yourself. `hnsw_rs` and `arroy` are pre-1.0. | Yes, with app-level work; "no second database" holds, since they are libraries | crates.io |

### 7.2 Mixed German + English stemming

A stemmer is per-language. For mixed content there are three general patterns:

- (a) Detect the language per page or paragraph and stem accordingly.
- (b) Index the same text twice, once with German stemming and once with English stemming, and query both fields with an OR, plus an optional unstemmed field for exact matches.
- (c) Index unstemmed text with n-grams or prefixes.

For language detection, pure-Rust crates exist: `whatlang` 0.18.0 (2025-10-16, MIT) and `lingua` 1.8.0 (2026-03-09, Apache-2.0).

| Candidate | German stemmer | English stemmer | Mixed-language mechanism | Meets (F)? |
| --- | --- | --- | --- | --- |
| **SQLite FTS5 (rusqlite)** | Not built in | `porter` (English only) | A custom FTS5 tokenizer through the `fts5_api` C API, wrapping `frostem`/`rust-stemmers`. It can emit both stems as colocated tokens (FTS5 supports synonyms through `FTS5_TOKEN_COLOCATED`). Or use two FTS5 columns or tables, or pre-stem in the app. rusqlite has no safe tokenizer API (issue #689, open since 2020). | Only with custom code (unsafe FFI, or app-side pre-stemming) |
| **Tantivy** | `Language::German` (Snowball, through `frostem`) | `Language::English` | Register two analyzers and write `body_de` and `body_en` fields. `QueryParser` searches both fields with per-field boosts. This is the textbook approach. | **Yes** |
| **Turso FTS** | **No** | **No** (no stemming tokenizers at all; the `SUPPORTED_TOKENIZERS` list is fixed) | None | **No** |
| **libSQL** | Same as SQLite FTS5 | Same | Same as SQLite (custom tokenizer through the C API) | Only with custom code |
| **SurrealDB** | `snowball(german)` | `snowball(english)` | One analyzer per FULLTEXT index. Whether two FULLTEXT indexes on the same field are allowed and addressable separately in `@N@` matches is **UNVERIFIED**. The safe pattern is to duplicate the text into `body_de`/`body_en` fields with one index each. Open FTS issues: 30, including #7290 "BM25 search::score(0) returns 0.0 on fresh databases" and #7014 (`@N@` ignores bound parameters). | **Yes**, with duplicated fields |
| **LadybugDB FTS** | `german` | `english`/`porter` | "Multiple FTS indexes on the same table must have different names". Whether two indexes on the same property with different stemmers are allowed is **UNVERIFIED**. Otherwise, duplicate the property. | Probably yes (**UNVERIFIED**) |
| **HelixDB / Grafeo** | Not documented (Grafeo: "Unicode tokenizer and stop word removal") | Not documented | n/a | **UNVERIFIED / likely no** |

---

## 8. Comparison table

| | rusqlite + FTS5 | rusqlite + Tantivy (FTS sidecar index) | Turso (`turso`) | libSQL | SurrealDB embedded | LadybugDB (`lbug`, see other doc) | redb/fjall + Tantivy |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Latest | 0.40.2 (SQLite 3.5x) | 0.40.2 + 0.27.0 | 0.8.3 (pre-1.0) | 0.10.0 | 3.3.4 | 0.21.2 | 4.4.0 / 3.1.12 + 0.27.0 |
| License | MIT + SQLite PD | MIT | MIT | MIT | **BSL 1.1** → Apache-2.0 on 2030-01-01 | MIT | MIT/Apache |
| Pure Rust / build | No (sqlite3.c; 46 s, 2.4 MB) | sqlite3.c + Rust (≈ +49 s, +0.6 MB) | Mostly (mimalloc/zstd C; 134 s, 23 MB) | No (C fork) | Rust engine with SurrealKV but `aws-lc-sys`; **398 s, 108 MB** | No (C++; prebuilt download) | Yes |
| Graph, 3 hops / backlinks | Recursive CTE | Recursive CTE | Recursive CTE, **open data-loss bug #8726** | Recursive CTE | Native `->e->`, `{1..3}` | Native Cypher | App code |
| **FTS: DE + EN stemming (F)** | Custom C-API tokenizer or pre-stemming only | **Yes** (two analyzers/fields) | **No** | Custom tokenizer only | **Yes** (`snowball`, duplicate fields) | Likely yes (27 stemmers; multi-index **UNVERIFIED**) | **Yes** |
| FTS ranking | bm25 | BM25 | BM25 (experimental) | bm25 | BM25 (30 open FTS issues) | BM25 | BM25 |
| **Vector (V)** | sqlite-vec = **pre-v1, stalled** → brute force only | Brute force in SQLite, or `hnsw_rs`/`arroy` library | Exact only; ANN on roadmap | DiskANN, built in (maintenance-mode codebase) | HNSW + DiskANN built in (17 open HNSW bugs, incl. wrong-result #7544) | HNSW extension (pre-1.0, runtime-loaded) | `hnsw_rs`/`arroy` library (pre-1.0) |
| Concurrency | WAL: 1 writer, N readers | same; Tantivy: 1 writer | WAL; MVCC **not prod-ready** | WAL | MVCC | Serializable, 1 RW process | redb: 1 writer; fjall: MVCC |
| Backup while running | Backup API, `VACUUM INTO` | + copy/snapshot Tantivy dir (rebuildable) | `VACUUM INTO`; no backup API | Backup API | `db.export()` SurrealQL dump | `EXPORT DATABASE` in-process | Savepoints/snapshots + Tantivy |
| Format stability | Very high | SQLite high; Tantivy index rebuildable | SQLite format; Turso-only features opt-in | SQLite format | Majors break (2→3 export/import); SurrealKV "not frozen" | Changes between minors | Stable within major |
| Maturity signal | Decades | Both mature/active | Very active, 1,000+ open issues | Maintenance mode | Active; 3 majors in 2.5 years | Bus factor ≈1 | Active |
