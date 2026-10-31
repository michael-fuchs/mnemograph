# LLM calls run on the server, through a built-in agent

**Status:** superseded by ADR 0008 and the assistant architecture (2026-10-31): mnemograph has no chat, no import and no built-in agent; its remaining LLM use (embeddings) goes through a shared LLM gateway service.

Sources need abstracts and must be worked into pages, and the owner wants a chat with the wiki in the web UI. Instead of leaving all LLM work to external agents, mnemograph calls LLMs itself. Everything it writes this way is authored by the **built-in agent** (`agent:mnemograph`), which follows the same rules as every agent: it never deletes and is subject to write limits. Each of its writes records the model used and its **cause**, the import or conversation that led to it, so the owner can revert a whole import or conversation at once (as new revisions; its notes are deleted). This is what makes direct page edits after an import acceptable without a review queue.

Providers sit behind mnemograph's own small provider trait (chat with tools, embeddings, OCR, usage); the first implementation uses the `genai` crate with Mistral (EU), with models configurable per feature, and a local provider can be added later. Embeddings are derived data keyed by model, so switching provider re-embeds in the background. A configurable monthly budget (default 10 EUR) stops LLM calls when reached; search then falls back to full-text and jobs wait. All content is processed by the configured provider; nothing is excluded, and the docs say so.

## Considered Options

- **Agents do all LLM work, the server stays deterministic:** no API key or running cost on the server, but no automatic import and no chat in the web UI.
- **Owner as author of owner-triggered LLM writes:** simpler history, but LLM text would look like the owner's own.
- **A separate author kind for services:** precise, but a new concept and new permission rules for what is just another agent.
- **Proposals the owner approves before they touch pages:** safest, but slows every import down to the owner's review; revert per cause gives the same safety afterwards.
- **A hosted embedding API versus a local model:** hosted first for speed and RAM on the small server; the trait keeps local models open.
- **`rig-core` instead of `genai`:** documents Mistral embeddings, but is heavier and breaks its API with every minor release.

## Consequences

The server holds an API key and has running costs, content leaves the server for the provider, and every write path of the built-in agent must carry its cause.
