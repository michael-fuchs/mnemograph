# Sources are separate from pages

Uploaded files and web resources become **sources**, a concept beside pages rather than a page type. A page is curated knowledge that changes through revisions; a source is external content that must stay as it was received, so it consists of unchanging **snapshots** (refreshing a URL adds a new snapshot when the content changed) plus metadata (fixed core fields and a free key-value map), extracted text and an abstract. Pages and notes cite sources with `[[source:Title]]`, a namespace of its own, so a page and a source about the same subject (a page "Rauthy" and the source "Rauthy documentation") never collide. This complements ADR 0001: pages remain the only unit of curated knowledge and the only target of links.

## Considered Options

- **A source as a page of type `source`:** no new concept, but metadata would live in Markdown, the extracted text and snapshots would need an attachment mechanism anyway, and agents could rewrite what should stay unchanged.
- **One namespace for pages and sources:** one syntax, but colliding titles.
- **Citing by URL or file hash:** no new syntax, but files have no natural URL and citations become unreadable.

## Consequences

New MCP tools (`add_source`, `read_source`) grow the tool surface, search and the graph view must handle a second kind of node, and export must include sources and their files.
