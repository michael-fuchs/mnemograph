# AGENTS.md — mnemograph

A lean, self-hosted knowledge base that is both a human-readable wiki and a machine-readable agent memory, shipped as a single Rust binary.

## Status

v1 is implemented for local use (agents via MCP with OAuth, owner web UI, CLI); see `README.md` for running it and `dev/setup-rauthy.sh` for the local identity provider. Deployment on the server is not done yet. The plan is `docs/plans/mnemograph-v1.md`; it supersedes `docs/plans/mnemograph-initial.md` where they disagree (the initial plan remains as background). Vocabulary is in `GLOSSARY.md`, decisions with trade-offs in `docs/adr/`, and sourced library facts in `docs/research/`. `Scratch.md` is a private, git-ignored notes file. When ideas are refined, update the plan documents under `docs/plans/` rather than scattering decisions elsewhere.

## Intended architecture (v1)

- **Single binary:** axum HTTP server + read-only web UI (askama + Datastar) + MCP server (`rmcp`, Streamable HTTP at `/mcp`) + CLI (`serve`, `export`, `delete`, `reindex`).
- **Storage:** SQLite (`rusqlite`, bundled) is the only source of truth; Tantivy is a derived, rebuildable full-text index with German and English stemming (ADR 0003).
- **Domain:** pages (no separate entities, ADR 0001) with titles, aliases, optional page type and Markdown body; untyped `[[Title|label]]` links; implicit stubs; append-only notes; revisions with authors.
- **Auth:** Rauthy as OIDC provider (local docker-compose here; production instance owned by `../server-infra`). Agents use authorization code + PKCE with one pre-registered client per installation; mnemograph validates JWTs itself and serves Protected Resource Metadata (ADR 0002).
- **Cargo workspace:** `mnemograph-core` (domain + storage) and `mnemograph` (binary).

## Design constraints to preserve

- Agents never delete; deletion is owner-only (CLI in v1).
- Every revision and note records its author from the token, never from request content.
- No free-form query language exposed to agents.
- Keep the MCP tool surface small (v1: `search`, `read_page`, `write_page`, `add_note`, `related`); no feature creep toward a Notion/Outline clone.
- Data must stay exportable (`mnemograph export`), and search indexes must always be rebuildable from SQLite.
- This repo is public: describe infrastructure requirements generically; hostnames, IPs and client IDs belong in `../server-infra`.

## Open decisions

Deferred items are listed at the end of `docs/plans/mnemograph-v1.md`.

## Agent skills

### Issue tracker

Issues live in GitHub Issues for `michael-fuchs/mnemograph` (public), managed via `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
