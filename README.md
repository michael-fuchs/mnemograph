# mnemograph

A lean, self-hosted knowledge base that is both a human-readable wiki and a long-term memory for AI agents, shipped as a single Rust binary.

**Status:** early development. The plan is in [`docs/plans/mnemograph-v1.md`](docs/plans/mnemograph-v1.md), the vocabulary in [`GLOSSARY.md`](GLOSSARY.md) and design decisions in [`docs/adr/`](docs/adr/).

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

`cargo run -- reindex` rebuilds the search index on demand. `cargo run -- export <dir>` writes every page as a Markdown file with a YAML header (title, type, aliases) to `<dir>/pages/` and all notes to `<dir>/notes.jsonl`.

There is no authentication yet, so the server refuses to listen on anything but a loopback address.

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

## Development

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

[MIT](LICENSE)
