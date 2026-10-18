# SQLite as source of truth, with derived search indexes

The initial plan used LadybugDB as the single store for graph, full-text and vector search. We chose SQLite (`rusqlite`, bundled) as the only source of truth for pages, revisions, aliases, notes and links; graph traversal uses recursive CTEs. Full-text search runs on Tantivy, with every text indexed once with a German and once with an English stemmer. Vector search (later) stores embeddings in SQLite and searches them exactly, adding a rebuildable ANN index only if that becomes too slow. Search indexes are derived data: they are updated after each SQLite commit and rebuilt from SQLite whenever they are missing, outdated or corrupt.

## Considered Options

- **LadybugDB:** effectively a single maintainer, an on-disk format that changes between minor releases (upgrade via export/import), and FTS/vector extensions downloaded at runtime.
- **SurrealDB embedded:** BSL 1.1 license until 2030, no in-place upgrades between major versions, a SurrealKV format that is not yet frozen, an open HNSW corruption bug, and very slow builds (about 6.6 minutes, 108 MB binary).
- **Turso:** no stemming, no ANN index, and an open recursive-CTE bug that matches our graph query.
- **SQLite FTS5 alone:** no German stemming.

See `docs/research/2026-10-storage-options.md`.

## Consequences

Two engines in one binary, and the Tantivy index can lag behind SQLite after a crash. A consistency check on startup must detect that and reindex.
