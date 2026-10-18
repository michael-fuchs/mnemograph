# AGENTS.md — mnemograph

A lean, self-hosted knowledge base that is both a human-readable wiki and a machine-readable agent memory, shipped as a single Rust binary.

## Status

Planning phase — no code yet. The current concept is `docs/plans/mnemograph-initial.md` (draft, not final). `Scratch.md` is a free-form notes file. When ideas are refined, update or add plan documents under `docs/plans/` rather than scattering decisions elsewhere.

## Intended architecture (from the initial plan)

- **Single binary:** axum HTTP server + server-rendered web UI (askama + htmx) + MCP server (`rmcp`, Streamable HTTP at `/mcp`) + embedded LadybugDB (`lbug` crate) for graph, full-text and later vector search. No separate DB process.
- **Documents + graph on the same data:** documents (Markdown body + YAML frontmatter) and entities/relations (`DOCUMENT`, `ENTITY`, `TAG`, `SOURCE`; edges `MENTIONS`, `RELATED_TO` with a `relation` property, `DESCRIBES`, `HAS_TAG`, `CITED_BY`).
- **"mnemograph-flavored" Markdown** (Comrak): `[[Wikilinks]]` create `MENTIONS` edges, `[[Entity|relation]]` creates typed relations, callouts, Mermaid, tables/task lists/footnotes.
- **Agent-first:** agents are the primary users via MCP; the web UI is for the owner to browse and curate.
- **Auth:** Rauthy as OIDC IdP (separate container, provided by `../server-infra`). Browser uses Authorization Code flow; each agent gets its own client-credentials client so it can be revoked individually. JWTs validated locally via JWKS.
- **Planned Cargo workspace:** `mnemograph-core` (domain), `mnemograph-store` (LadybugDB adapter, Cypher migrations), `mnemograph-mcp` (MCP tools), `mnemograph-web` (axum/UI/OIDC; the binary).

## Design constraints to preserve

- Agents never delete; deletion is web-UI-only by the owner.
- No free-form Cypher from agents — `query_graph` only exposes a whitelist of read-only query templates.
- Keep the MCP tool surface small; no feature creep toward a Notion/Outline clone.
- Data must stay exportable (Cypher/Parquet export, optional Markdown/git mirror).

## Open decisions

See §9 of the initial plan (Vibe MCP integration, wikilink vs. LLM extraction, Markdown mirror, coexistence with Outline).

## Agent skills

### Issue tracker

Issues live in GitHub Issues for `michael-fuchs/mnemograph` (public), managed via `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
