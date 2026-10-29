# mnemograph

A lean, self-hosted knowledge base that is both a human-readable wiki and a long-term memory for AI agents, shipped as a single Rust binary.

**Status:** v1 feature-complete for local use; deployment on a server is next. The plan is in [`docs/plans/mnemograph-v1.md`](docs/plans/mnemograph-v1.md), the vocabulary in [`GLOSSARY.md`](GLOSSARY.md) and design decisions in [`docs/adr/`](docs/adr/).

## Running locally

```sh
cargo run -- serve
```

This creates `mnemograph.db` (the data) and `mnemograph.db.search/` (the search index, rebuilt from the data whenever it is missing or outdated) in the current directory and listens on `http://127.0.0.1:8080`. Options (also settable via environment):

| Option     | Environment         | Default          |
| ---------- | ------------------- | ---------------- |
| `--db`     | `MNEMOGRAPH_DB`     | `mnemograph.db`  |
| `--listen` | `MNEMOGRAPH_LISTEN` | `127.0.0.1:8080` |
| `--page-types` | `MNEMOGRAPH_PAGE_TYPES` | `project,technology,decision,person,tool,concept` |
| `--oidc-issuer` | `MNEMOGRAPH_OIDC_ISSUER` | none: authentication disabled |
| `--public-url` | `MNEMOGRAPH_PUBLIC_URL` | `http://localhost:8080` |
| `--required-scopes` | `MNEMOGRAPH_REQUIRED_SCOPES` | `mnemograph` |
| `--web-client-id` | `MNEMOGRAPH_WEB_CLIENT_ID` | `mnemograph-web` |
| `--web-client-secret` | `MNEMOGRAPH_WEB_CLIENT_SECRET` | none: public client (PKCE only) |
| `--owner-group` | `MNEMOGRAPH_OWNER_GROUP` | `mnemograph_owner` |
| `--max-page-bytes` | `MNEMOGRAPH_MAX_PAGE_BYTES` | `204800` (200 KiB) |
| `--max-note-bytes` | `MNEMOGRAPH_MAX_NOTE_BYTES` | `4096` (4 KiB) |
| `--writes-per-minute` | `MNEMOGRAPH_WRITES_PER_MINUTE` | `60` per agent |

`cargo run -- reindex` rebuilds the search index on demand. `cargo run -- export <dir>` writes every page as a Markdown file with a YAML header (title, type, aliases) to `<dir>/pages/` and all notes to `<dir>/notes.jsonl`. `cargo run -- delete page <title>` and `cargo run -- delete note <id>` permanently delete a page (with its revisions and aliases; links to it become stubs) or a note, after confirmation (`--yes` skips it). Agents cannot delete anything. These commands need exclusive access, so stop `serve` first.

Without `--oidc-issuer`, authentication is disabled: every agent is anonymous and the server refuses to listen on anything but a loopback address.

## Authentication

Agents authenticate with OAuth (authorization code + PKCE) at an OIDC provider; mnemograph validates their access tokens and records the agent as the author of everything it writes (see [ADR 0002](docs/adr/0002-agents-use-authorization-code-with-pre-registered-clients.md)). For local development, run [Rauthy](https://github.com/sebadob/rauthy) in Docker:

```sh
dev/setup-rauthy.sh
```

It generates a local config with random secrets in `dev/rauthy/` (git-ignored; the admin password is in `dev/rauthy/secrets.env`), starts Rauthy on `http://localhost:8180`, and registers:

- the scope `mnemograph` and the public client `claude-code` for agents,
- the group `mnemograph_owner` with the admin in it, and the confidential client `mnemograph-web` for the web UI; mnemograph's settings for it go to `dev/rauthy/mnemograph.env`.

Then:

```sh
set -a; . dev/rauthy/mnemograph.env; set +a
cargo run -- serve
claude mcp add --transport http --client-id claude-code mnemograph http://localhost:8080/mcp
claude mcp login mnemograph
```

Log in as `admin@localhost` in the browser that opens. Claude Code closes its callback listener right after receiving the code, so the browser may afterwards show a connection error even though the terminal reports "Authenticated". If the browser cannot reach the callback at all, paste the URL from its address bar into the terminal prompt within a minute.

## Web UI

With authentication on, `http://localhost:8080/` asks the owner to log in at the OIDC provider (authorization code flow with PKCE). Only members of the owner group get in; the session is kept for 7 days in a cookie, and the database stores only a hash of it. The UI is read-only:

- the page list with recent notes;
- pages with rendered Markdown (tables, task lists, footnotes, alerts, Mermaid diagrams), links, backlinks and notes, and stubs with what links to them;
- a graph of each page's and stub's neighborhood: everything within 1, 2 or 3 links (like `related`), with stubs and page types marked; clicking a node opens it;
- search over pages and notes, optionally limited to a page type;
- each page's history with author and time of every revision, and any revision rendered as it was.

The page list, pages and histories update themselves when agents write (via [Datastar](https://data-star.dev) and server-sent events). Raw HTML in page bodies is never rendered. Without `--oidc-issuer` the UI is open to everyone on the loopback address.

"Log out" ends the mnemograph session; the provider's own session stays, so logging in again does not ask for the password.

Access tokens must be signed by the issuer, have `typ: at+jwt`, carry `<public-url>/mcp` as audience (RFC 8707) and the required scopes. Clients without a token get a `401` pointing to the Protected Resource Metadata at `/.well-known/oauth-protected-resource/mcp` (RFC 9728). To stop Rauthy: `docker compose -f dev/compose.yaml down` (add `-v` to delete its data).

## Connecting Claude Code

```sh
claude mcp add --transport http mnemograph http://localhost:8080/mcp
```

Claude Code then has these tools:

- `write_page`: create a page from a title and a Markdown body, or change one by sending the complete new body with the revision it is based on. Optionally sets the page type, the aliases, or a new title (the old title stays an alias). `[[Title]]` and `[[Title|label]]` link to other pages; a title no page has yet is a stub.
- `read_page`: read a page by title or alias, with its current revision, the titles it links to, the pages linking to it and its most recent notes.
- `search`: full-text search over pages and notes, understanding German and English word forms, optionally limited to one page type; hits come with a snippet.
- `related`: the pages and stubs within 1–3 links of a page (default 2), following links in both directions, nearest first.
- `add_note`: record a short, timestamped, append-only note; its `[[links]]` attach it to the pages (or stubs) it concerns.

Titles and aliases are compared case-insensitively, with whitespace collapsed.

### Telling agents to use it

mnemograph sends usage instructions to every agent that connects. To make an agent use it as *the* memory for a repository, add this to the repository's `CLAUDE.md` (or `AGENTS.md`) and replace `<project>` with the title of the project's page:

```markdown
## Memory

This project's long-term memory is the `mnemograph` MCP server; its page for this project is `<project>`.
Use it instead of local memory files.

- Before starting a task, search mnemograph for the task's subject and read the `<project>` page.
- Record decisions, findings and notable events with `add_note` as they happen, saying what and why,
  and link `[[<project>]]` plus the pages they concern.
- Keep durable knowledge in pages (`write_page`); update existing pages instead of creating near-duplicates.
```

### Filing conventions

How knowledge is structured (hub pages per project with a timeline, decision pages, notes versus pages) is up to you, so it lives in mnemograph itself: on the page **Using mnemograph**. As soon as that page exists, the instructions every agent receives on connecting tell it to read the page before its first write; renaming the page keeps working, since the old title stays an alias. Edit it like any other page to change the conventions.

A fresh installation has no such page. [`docs/using-mnemograph.md`](docs/using-mnemograph.md) is a starting point: ask an agent to create the page from it.

## Container image

Every push to `main` builds a public image, `ghcr.io/michael-fuchs/mnemograph`, tagged with the branch and the commit (`main`, `sha-<commit>`); version tags `v*` add the version. Pin it by digest when deploying.

The image is based on distroless (glibc, with CA certificates), runs as the non-root user 65532 and needs no capabilities. The database and the search index live in the volume `/data`, which must be writable by that user; everything else can be read-only. It listens on `0.0.0.0:8080` and, like outside a container, refuses to do so without `MNEMOGRAPH_OIDC_ISSUER`. Configure it with the environment variables above; the issuer is the exact `issuer` value of the provider's discovery document (for Rauthy this ends in `/auth/v1/`).

```sh
docker build -t mnemograph .
docker run --rm -v mnemograph-data:/data -p 127.0.0.1:8080:8080 mnemograph   # loopback only: needs auth settings to listen on 0.0.0.0
```

The CLI commands run in the same image, with the server stopped: `docker run --rm -v mnemograph-data:/data mnemograph export /data/export`.

## Development

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

[MIT](LICENSE)
