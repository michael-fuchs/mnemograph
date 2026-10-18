# Plan: mnemograph v1

The scope of the first working version, from the design session on 2026-10-18. It supersedes `mnemograph-initial.md` wherever the two disagree. Vocabulary follows `GLOSSARY.md`; decisions with lasting trade-offs are recorded in `docs/adr/`.

## Goal

mnemograph runs locally on the owner's machine with real OAuth. Claude Code, the only agent in v1, uses it as long-term memory over MCP. The owner browses what agents wrote in a read-only web UI. There is no server yet, but the setup is production-shaped, so deployment later only adds TLS and a reverse proxy (provided by `../server-infra`).

## Domain

- **Pages are the only linkable unit.** Entities do not exist separately ([ADR 0001](../adr/0001-pages-and-entities-are-one-concept.md)).
  - A page has a stable internal ID, a unique title, aliases, an optional page type and a Markdown body.
  - Titles and aliases are compared case-insensitively with whitespace normalized. Renaming keeps the old title as an alias, so links never break.
  - Page types come from a fixed list in the config: `project`, `technology`, `decision`, `person`, `tool`, `concept`. An unknown type is rejected with the list of allowed types.
- **Links** are `[[Title]]` or `[[Title|label]]`, where the part after the pipe is a display label. Links are untyped in v1.
- **Stubs are implicit.** A stub is a link title that resolves to no page; the first `write_page` turns it into a page.
- **Notes** are short, timestamped and append-only, and link to pages via `[[...]]`. Notes stay unchanged when their content is worked into a page; pages list related notes as context.
- **Revisions:** every page change creates one. Revisions are kept forever.
- **Author:** every revision and note records its author (the owner, or a specific agent registration). The author comes from the token, never from request content.
- **Deletion:** only the owner deletes, via the CLI.
  - Deleting a page removes it with all revisions and aliases; incoming links remain and the title becomes a stub.
  - Deleting a note removes only that note. There is no trash.

## Agent interface (MCP)

The MCP server is `rmcp` Streamable HTTP at `/mcp`. It accepts every spec revision rmcp supports.

| Tool         | Purpose                                                                                                  |
| ------------ | -------------------------------------------------------------------------------------------------------- |
| `search`     | Full-text search over pages and notes, optional page-type filter                                         |
| `read_page`  | A page by title or alias, with links, backlinks and recent notes                                         |
| `write_page` | Create or update a page: title, type, aliases, Markdown body. Updates must give the base revision; a stale base is rejected with the current revision |
| `add_note`   | Append a note; its `[[links]]` attach it to pages                                                        |
| `related`    | A page's link neighborhood up to n hops                                                                  |

- **Body format:** Markdown without frontmatter. Supported extensions are what comrak already provides: tables, task lists, footnotes, the five GitHub alert types and Mermaid code blocks.
- **Guidance:** the MCP `instructions` field tells agents to search before a task, record decisions as notes, link the project page and prefer existing titles. The README offers a short snippet for repo `CLAUDE.md` files.
- **Permissions and limits:** agents have the `read` and `write` scopes; the owner additionally has `delete`. Hard, configurable limits: 200 KB per page body, 4 KB per note, 60 writes per minute per agent.

## Authentication

See [ADR 0002](../adr/0002-agents-use-authorization-code-with-pre-registered-clients.md).

- **Identity provider:** Rauthy, run locally from `docker-compose` in this repo.
- **Agents:** one pre-registered Rauthy client per agent installation, using the authorization code flow with PKCE and a localhost callback. Dynamic client registration stays off.
- **Token validation:** mnemograph is the OAuth resource server. It validates JWTs locally against Rauthy's JWKS, accepting EdDSA signatures and `typ: at+jwt`, and checks issuer, audience (its resource URL), expiry and scopes.
- **Self-implemented parts:** RFC 9728 Protected Resource Metadata and the `401` + `WWW-Authenticate` challenge, because `rmcp` has no server-side auth.
- **Browser login:** OIDC authorization code flow, open only to members of the configured owner group.
- **Transport:** plain HTTP on localhost in v1. In production, TLS terminates at the reverse proxy.

## Storage

See [ADR 0003](../adr/0003-sqlite-source-of-truth-with-derived-indexes.md).

- SQLite (`rusqlite`, bundled) is the only source of truth, with a single writer and recursive CTEs for graph traversal.
- Tantivy provides full-text search with German and English stemmed fields. The index is updated after each commit and rebuilt from SQLite on mismatch (startup check, `reindex`).
- Vector search is not in v1. Later: embeddings in SQLite with exact search, and an ANN index only if needed.

## Web UI

Read-only: browse pages, notes, links, backlinks and stubs; search; view revision history. Built with askama templates and Datastar, which can also push live updates when agents write. The Mermaid and Datastar scripts are served as static files.

## CLI

- `serve`
- `export <dir>`: pages as Markdown with a YAML header (title, type, aliases), notes as JSONL
- `delete`
- `reindex`

## Code layout and libraries

- **Crates:** `mnemograph-core` (domain and storage behind a deep interface) and `mnemograph` (binary: axum, MCP, web UI, auth, CLI). Split further only when a seam actually hurts.
- **Libraries:** `axum` 0.8, `tokio`, `tower`, `rmcp` 3.5, `rusqlite` (bundled), `rusqlite_migration`, `tantivy` 0.27, `comrak`, `jsonwebtoken` 11, `openidconnect` 4, `tower-sessions` (SQLite store), `askama` 0.16 + `askama_web`, `datastar` (Rust SDK), `serde`, `tracing`, `clap`, `thiserror`/`anyhow`.
- **CI:** GitHub Actions running fmt, clippy and tests; integration tests against a temporary SQLite database. The Dockerfile waits for the deployment phase.

Version facts and sources: `docs/research/2026-10-initial-stack-facts.md`, `docs/research/2026-10-storage-options.md`.

## Deferred

Typed links (likely via structured fields), vector search, Claude.ai/Desktop and Vibe connectors, deployment on the server, git mirror, graph view, editing in the web UI, MCP prompts.
