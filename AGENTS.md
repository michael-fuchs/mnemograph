# AGENTS.md — mnemograph

A lean, self-hosted knowledge base that is both a human-readable wiki and a machine-readable agent memory, shipped as a single Rust binary.

## Status

v1 is implemented for local use (agents via MCP with OAuth, owner web UI, CLI); see `README.md` for running it and `dev/setup-rauthy.sh` for the local identity provider. The graph view and statistics of v2 are built; mnemograph is one service of the owner's personal assistant (independent services sharing identity provider, LLM gateway and a central configuration of spheres; documented privately in `../server-infra/docs/assistant/`). What remains for mnemograph (spheres, shared crates, vector search, citations) is `docs/plans/mnemograph-v2.md`; ADRs 0005–0007 are superseded. The v1 plan is `docs/plans/mnemograph-v1.md`; it supersedes `docs/plans/mnemograph-initial.md` where they disagree (the initial plan remains as background). Vocabulary is in `GLOSSARY.md`, decisions with trade-offs in `docs/adr/`, and sourced library facts in `docs/research/`. `Scratch.md` is a private, git-ignored notes file. When ideas are refined, update the plan documents under `docs/plans/` rather than scattering decisions elsewhere.

## Intended architecture (v1)

- **Single binary:** axum HTTP server + read-only web UI (askama + Datastar) + MCP server (`rmcp`, Streamable HTTP at `/mcp`) + CLI (`serve`, `export`, `delete`, `reindex`).
- **Storage:** SQLite (`rusqlite`, bundled) is the only source of truth; Tantivy is a derived, rebuildable full-text index with German and English stemming (ADR 0003).
- **Domain:** pages (no separate entities, ADR 0001) with titles, aliases, optional page type and Markdown body; untyped `[[Title|label]]` links; implicit stubs; append-only notes; revisions with authors.
- **Auth:** Rauthy as OIDC provider (local docker-compose here; production instance owned by `../server-infra`). Agents use authorization code + PKCE with one pre-registered client per installation; mnemograph validates JWTs itself and serves Protected Resource Metadata (ADR 0002).
- **Cargo workspace:** `mnemograph-core` (domain + storage) and `mnemograph` (binary).

## Planned additions (v2)

- **Spheres** (ADR 0008): pages and notes carry spheres; each agent registration is bound to a set of spheres carried in its tokens, and sees only pages and notes of those spheres.
- **Shared crates:** auth, sphere filtering and configuration, search, the LLM gateway client and UI basics move to a separate public repository used by all of the assistant's services.
- **Search:** hybrid full-text plus vector search inside the existing `search` tool; vectors are derived data in SQLite, computed through the assistant's LLM gateway.
- **Citations:** normal links to other services' items (documents, appointments), recognised by host.
- **Layout:** only wiki features live here; chat and documents are other services (ADR 0004, narrowed).

## Design constraints to preserve

- Agents never delete; deletion is owner-only (CLI in v1).
- An agent sees and changes only pages and notes of its registration's spheres; the boundary is enforced in the core, never left to the agent.
- Every revision and note records its author from the token, never from request content.
- No free-form query language exposed to agents.
- Keep the MCP tool surface small (`search`, `read_page`, `write_page`, `add_note`, `related`); no feature creep toward a Notion/Outline clone.
- Data must stay exportable (`mnemograph export`), and search indexes must always be rebuildable from SQLite.
- This repo is public: describe infrastructure requirements generically; hostnames, IPs and client IDs belong in `../server-infra`.

## Open decisions

Deferred items are listed at the end of `docs/plans/mnemograph-v2.md`.

## Agent skills

### Issue tracker

Issues live in GitHub Issues for `michael-fuchs/mnemograph` (public), managed via `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
