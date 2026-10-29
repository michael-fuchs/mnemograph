# Plan: mnemograph v2

The architecture for the second round of features, from the design session on 2026-10-28/29. It builds on `mnemograph-v1.md`, which stays valid where this plan says nothing. Vocabulary follows `GLOSSARY.md`; decisions with lasting trade-offs are in `docs/adr/` (0004–0007). Sourced facts are in `docs/research/2026-10-v2-providers-and-storage.md`.

## Goal

Six epics: a graph view, usage statistics, vector search for agents, sources (imported documents and web references with abstracts and metadata), and a chat with the wiki in the web UI. They need LLM calls on the server, so v2 adds the built-in agent, a provider layer and background jobs. The target server has 2 vCPU, 4 GB RAM and no GPU.

## System layout

- **mnemograph stays one binary** for all wiki features; non-wiki agent functions become separate services behind the shared reverse proxy and identity provider ([ADR 0004](../adr/0004-wiki-monolith-with-separate-services.md)).
- **Garage** runs beside it as the object store for source files ([ADR 0007](../adr/0007-garage-s3-for-source-files.md)). The local dev compose file gets a Garage container too.
- **The LLM provider** is reached over HTTPS; nothing else new is exposed.

## Built-in agent and providers

See [ADR 0005](../adr/0005-llm-calls-on-the-server-through-a-built-in-agent.md).

- **The built-in agent** (`agent:mnemograph`) authors everything the server's LLM writes: abstracts, page changes and notes from imports, and writes from conversations.
  - It follows the agent rules: no deleting, write limits.
  - Every write records the model and its cause (an import or a conversation).
- **Revert a cause:** the owner can undo everything one import or conversation did. Each page touched gets a new revision with its body from before the cause; notes with that cause are deleted. CLI first, web UI later.
- **Provider trait:** mnemograph's own small trait covers chat with tools, embeddings, OCR and usage reporting.
  - Implementation: the `genai` crate. OCR uses a direct HTTP call.
  - First provider: Mistral via its EU endpoint, with training opted out in the console. A local provider can be added later.
- **Models are configurable per feature.** Defaults:

  | Feature | Default model |
  |---|---|
  | Chat, import integration | Mistral Large 4 |
  | Abstracts, metadata | Mistral Small 4 |
  | Embeddings | mistral-embed |
  | OCR (scanned or complex PDFs) | Mistral OCR 3 |

- **Budget:** configurable monthly limit, default 10 EUR.
  - The dashboard warns at 80 %.
  - At 100 % LLM calls stop: search falls back to full-text, and jobs wait until the next month or a higher limit.
- **Usage:** tokens, pages and estimated cost are recorded per provider, model and feature.
- **Background jobs:** embedding, conversion, fetching and abstracts run as durable jobs in a SQLite table, with an in-process worker, retries and backoff. Jobs survive restarts.
- **Privacy:** all content is processed by the configured provider; nothing is excluded. README and dashboard say so.

## Vector search

- **Chunks:** pages split by heading section, notes whole, source snapshots in chunks of about 500 tokens.
- **Storage:** vectors live in SQLite, keyed by model and content hash, and are searched exactly (brute force). They are derived data like the full-text index (ADR 0003).
  - Changing the model re-embeds in the background.
  - Unchanged chunks are never embedded twice.
  - An ANN index comes only if exact search gets too slow.
- **MCP:** the existing `search` tool merges full-text and vector hits with reciprocal rank fusion (k = 60). It falls back to full-text when embeddings are unavailable (provider down, budget reached, model change in progress). The tool surface stays the same.

## Sources

See [ADR 0006](../adr/0006-sources-are-separate-from-pages.md). Documents and external references are one concept and one epic.

- **What a source is:** an uploaded file or a web resource. It consists of snapshots, plus an abstract written by the built-in agent. Refreshing a URL adds a snapshot only when the content changed, and rewrites the abstract.
- **Metadata:**
  - Fixed core fields: title, kind (file or web), URL or file name, media type, language, authors, published date, fetched or uploaded at, size, content hash.
  - A free key-value map for everything else found (OpenGraph, schema.org, PDF info, DOI).
  - Core fields are filterable; everything is searchable.
- **Citations:** `[[source:Title]]` and `[[source:Title|label]]`, in a namespace separate from pages. Pages and sources show their citations in both directions.
- **Conversion to Markdown** happens in the binary:
  - PDF: `pdf_oxide`, with tables.
  - HTML: `htmd`.
  - Web main content and metadata: `dom_smoothie` and `webpage`.
  - Plain text and Markdown as they are; DOCX with a small reader.
  - Scanned or complex PDFs go to Mistral OCR.
- **Files:** stored in Garage, content-addressed by SHA-256, with a size limit per file. Export includes them.
- **Import flow:**
  1. store
  2. convert
  3. extract metadata
  4. abstract
  5. embed
  6. built-in agent integration: it changes pages and adds notes directly, citing the source. All of it carries the import as its cause.
- **Ways in:**
  - Owner: upload or URL in the web UI. These are the UI's first forms, so they come with CSRF protection.
  - Agents: MCP `add_source`, by URL or by file content (base64, size-limited), and `read_source`.
  - The chat.

## Chat

- **Conversations:** the owner talks to the built-in agent in the web UI.
- **Tools:** it uses the same tools as MCP, in-process, to answer with citations of pages and sources, and on request to write pages, notes or import URLs and attached files. It never deletes.
- **Streaming:** answers stream over Datastar SSE.
- **Storage:** conversations are stored in SQLite, visible only to the owner and deletable by the owner. A conversation is the cause of its writes.

## Graph view

- **Views:** a neighbourhood graph per page (1–3 hops, like `related`) and a whole-wiki graph filterable by page type.
- **Nodes:** pages, stubs and sources are differently styled nodes.
- **Rendering:** server-driven through Datastar, which delivers the graph data and live updates. One vendored, prebuilt standalone JS layout library (Cytoscape.js or similar) plus a few lines of glue does the drawing. No bundler, no npm build.

## Usage statistics

A dashboard in the web UI with four parts:

- **Knowledge growth:** pages, notes, sources, links and stubs over time; most-linked and orphaned pages.
- **Activity per author** over time, from revisions and notes.
- **Access log:** which agent read or searched what. Retention is configurable, default 90 days. Searches without hits show knowledge gaps.
- **LLM usage and cost** per provider, model and feature, against the budget.

## Epic order

1. **Graph view**: no LLM dependency.
2. **Statistics**: knowledge growth, activity, access log.
3. **Foundation**: provider trait with a `genai` spike for Mistral chat and embeddings, jobs, usage and cost tracking with budget, the LLM-cost panel.
4. **Vector search.**
5. **Sources**: Garage, conversion, import, web references, revert.
6. **Chat.**

## Requirements on server-infra

- **Garage container:** single node, SQLite metadata engine, fsync on, automatic metadata snapshots. Its backup is the snapshot plus the data directory, in the nightly backup.
- **Secrets:** the Mistral API key and the Garage access key.
- **mnemograph's memory limit** rises to about 512 MiB, for PDF conversion.
- **Outbound HTTPS** to the LLM provider.

## Deferred

From v1, still deferred: typed links, git mirror, editing in the web UI, MCP prompts. Also new:

- an ANN index;
- scheduled refresh of web sources;
- local LLM or embedding providers;
- revert and conversation management in the web UI beyond the basics;
- a shared crate for services.
