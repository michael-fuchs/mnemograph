# The wiki stays one binary; other agent functions are separate services

**Status:** accepted, narrowed (2026-10-31): the wiki keeps only wiki features; chat and documents moved out as well, following the assistant's rule of one service per kind of thing.

The owner plans to run more agent-facing functions on the server, some of them unrelated to the wiki. We keep every wiki feature, including the v2 epics (graph view, statistics, vector search, sources, chat), inside the single mnemograph binary, and give each non-wiki function its own service: its own repository, container and MCP endpoint, behind the shared reverse proxy and identity provider. Agents connect to each service separately. mnemograph is open source and focused; mixing private, unrelated functions into it would blur both its scope and its public repository. Code that services end up sharing (token validation, the LLM provider layer) can move into a small shared crate when the second service needs it, not before.

## Considered Options

- **One platform binary for everything:** a single deployment and MCP endpoint, but private functions inside the public wiki, one memory limit and one failure domain for all.
- **Microservices for the wiki epics too** (embedding, import, chat as services): independent deployment, but many moving parts on a 2 vCPU / 4 GB server and service-to-service authentication for no gain at one user's scale.

## Consequences

Each new service repeats the resource-server setup (Protected Resource Metadata, token validation, a pre-registered client per agent), and every agent needs one connector per service.
