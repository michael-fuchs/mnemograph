# Plan: mnemograph v2

The second round of features, revised on 2026-10-31. The first version of this plan (2026-10-29) put chat, document import and file storage into mnemograph. The owner then decided to build a personal assistant of independent services, one per kind of thing, sharing an identity provider, a reverse proxy, an LLM gateway and a central configuration of spheres. mnemograph is the wiki and agent memory among them. That architecture is documented privately with the server's infrastructure; this plan covers what it means for mnemograph. Vocabulary follows `GLOSSARY.md`; decisions are in `docs/adr/` (0008 is new; 0005–0007 are superseded).

## Done

- **Graph view** (#33, #34): each page's neighbourhood and the whole wiki, filterable by page type, live.
- **Statistics** (#37–#39): knowledge growth, activity per author, the access log of agents' reads and searches with knowledge gaps.

## Spheres (next)

[ADR 0008](../adr/0008-pages-and-notes-carry-spheres.md).

- **Pages and notes:** every page and note carries one or more spheres.
- **Registrations:** each agent registration (a pre-registered OAuth client) is bound to a set of spheres, and its access tokens carry them, for example as scopes. mnemograph checks them like the audience and the required scope.
  - An agent installation that works in several sphere sets has one registration per set, and a repository's MCP configuration picks one.
- **Visibility:** through a registration, an agent sees a page or note only if it shares a sphere with the registration. This holds for every tool, the graph, the statistics and the access log.
  - The owner's web login sees everything and can filter by sphere.
- **Writes:** new pages and notes get the registration's sphere. A registration with several spheres must name one.
  - Changing a page keeps its spheres unless the agent changes them within its own.
- **Links to invisible pages** show as stubs.
- **Sphere rules:** the owner's central configuration lists the spheres and their rules. A sphere whose content must not enter the wiki is refused.
- **Existing content:** existing pages and notes get a default sphere chosen at the upgrade.

## Shared crates

The parts other services need move into a separate public repository of crates, pinned by git tag:

- validating access tokens and serving Protected Resource Metadata;
- sphere filtering and reading the central configuration;
- the search engine (Tantivy with German and English stemming, vectors, fusion);
- the LLM gateway client;
- the web UI basics (stylesheet, header, charts).

mnemograph then depends on them like every other service.

## Vector search

- **Embeddings:** pages are embedded per heading section and notes whole. Vectors are stored in SQLite, keyed by model and content hash, and searched exactly ([ADR 0003](../adr/0003-sqlite-source-of-truth-with-derived-indexes.md)). They are derived data: a model change re-embeds in the background.
- **LLM gateway:** embeddings come from the assistant's LLM gateway, which holds the provider key, the budget and the rate limits. mnemograph keeps a durable job queue in SQLite for embedding work and waits when the gateway reports the budget used up.
- **Search:** the existing `search` tool fuses full-text and vector hits (reciprocal rank fusion). It falls back to full-text when vectors are unavailable.
- **Sphere rules** about LLM providers apply: content of a sphere that may not leave the server gets no vectors.

## Citations

Other services (documents, calendar) give each item a stable URL. A page or note cites one with a normal Markdown link. mnemograph recognises citations by the services' hosts from the central configuration, shows their kind, and lists a page's citations. Agents resolve a citation with the owning service's read tool. mnemograph stores no copies.

## Dropped from the first v2 plan

- Chat in the web UI: general-purpose clients do this.
- The built-in agent.
- Sources, import, and the `add_source` and `read_source` tools: now the document service.
- Garage.
- The LLM-cost panel moves to the gateway.

## Deferred

- typed links
- git mirror
- editing in the web UI
- MCP prompts
- an ANN index
