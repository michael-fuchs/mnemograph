# Initial stack facts (research, 2026-10-18)

Facts gathered to inform `docs/plans/mnemograph-initial.md`. These are facts, not recommendations.
Sources are primary (crates.io API, GitHub API/repos, official docs) unless marked otherwise.
Items marked **UNVERIFIED** could not be confirmed from a primary source.

Versions and dates are as of 2026-10-18.

---

## 1. LadybugDB / `lbug` crate

### Status and versions

| Fact | Value | Source |
| --- | --- | --- |
| Latest `lbug` crate | **0.21.2**, published 2026-10-01 (0.21.1 on 09-30, 0.21.0-pre on 09-29) | https://crates.io/crates/lbug |
| Crate license / MSRV / size | MIT, `rust-version = 1.81`, ~9.9 MB crate (bundles C++ sources) | crates.io API, https://github.com/LadybugDB/ladybug-rust/blob/main/Cargo.toml |
| Rust bindings repo | https://github.com/LadybugDB/ladybug-rust (4 stars, last push 2026-09-29) | GitHub API |
| Core repo | https://github.com/LadybugDB/ladybug: 1,858 stars, last push 2026-10-18, not archived | GitHub API |
| Core releases | v0.21.2 (2026-10-01), v0.21.1, v0.21.0 (09-29), v0.20.0–0.20.4 (Aug–Sep), v0.19.x (Jul–Aug), v0.18.x (Jul), v0.17.x (May–Jun), … v0.12.0 (2025-11-04). Roughly one minor release per month. | https://github.com/LadybugDB/ladybug/releases |
| Activity | 100+ commits in the 30 days before 2026-10-18 | GitHub API |
| Maintainers | Commits in the last 3 months are concentrated: `adsharma` 362, then `zahariash` 50, `aikins01` 25, `kory-io` 19, others ≤13. All-time top contributors (`andyfengHKU`, `acquamarin`, `ray6080`, `mewim`, …) come from the inherited Kùzu history. In practice this is a **bus factor of about 1**. | GitHub contributors/commits API |
| Self-description | "Ladybug is an embedded graph database… The database was formerly known as Kuzu." Features include native FTS and vector index, serializable ACID transactions, and Cypher. | https://github.com/LadybugDB/ladybug (README) |

### Kùzu and other forks

- **Kùzu is archived**: `kuzudb/kuzu` is archived on GitHub. The last release is v0.11.3 (2025-10-10), and the last `kuzu` crate is 0.11.3 (2025-10-10). The README says "Kuzu is working on something new! We are archiving the KuzuDB project". It also says that 0.11.3 "bundles many (but not all) of the extensions", and that users can otherwise host their own extension server. Source: https://github.com/kuzudb/kuzu
- Some sources say Apple acquired the company. This appears only in secondary sources (oneuptime blog). **UNVERIFIED**.
- **RyuGraph** (`predictable-labs/ryugraph`): a Kùzu fork by Predictable Labs. It has 147 stars, its last release is v25.9.2 (2025-12-06) and its last push was 2026-01-20. The `ryugraph` crate was last updated 2025-12-06. It looks dormant.
- **Bighorn** (`Kineviz/bighorn`): a Kùzu fork by Kineviz. It has 135 stars, no GitHub releases, and its last push was 2026-08-10.
- In stars, release cadence and crate publishing, LadybugDB is clearly the most active fork.

### Extensions (FTS, vector) — **important for offline/Docker**

- Extensions are **not bundled** into the Linux/macOS/Windows Rust builds. The extensions repo's `extension_config.cmake` statically links `fts`, `json`, `vector` and `algo` only for WASM, Android and Swift builds. For the other platforms the static-link list is commented out (`#set(EXTENSION_STATIC_LINK_LIST fts)`). Source: https://github.com/LadybugDB/extensions/blob/main/extension_config.cmake
- Extensions are shared libraries (`lib<name>.lbug_extension`). By default, `INSTALL fts;` downloads them at runtime from `OFFICIAL_EXTENSION_REPO = "https://extension.ladybugdb.com/"` (`src/include/extension/extension.h`). After that, each session/connection must run `LOAD fts;`, because "Extensions are scoped to the session within which they are loaded". Source: https://docs.ladybugdb.com/extensions/
- I verified the URL layout: `https://extension.ladybugdb.com/v0.21.0/linux_amd64/fts/libfts.lbug_extension` returns HTTP 200, while the same path with `v0.21.2` returns 404. The extension ABI version is pinned via `LBUG_EXTENSION_VERSION="0.21.0"` in the core `CMakeLists.txt`, not the patch version.
- Ways to run offline:
  1. Self-host the extension repo with `docker run -p 8080:80 ghcr.io/ladybugdb/extension-repo:latest`.
  2. `LOAD EXTENSION '/path/libfts.lbug_extension'` loads from a file path; the docs show this for custom extensions.
  3. Build from source with `STATICALLY_LINKED_EXTENSIONS` (a cmake option; not exposed by the `lbug` build.rs). **UNVERIFIED** that this works through the crate.
- The Rust crate doc says: "By default, binaries created using this library will not work with Lbug's extensions… add `println!("cargo:rustc-link-arg=-rdynamic");` to your build.rs… Not doing this will produce undefined symbol errors when the extension is loaded." It also says to leave `LBUG_LOCALIZE_BUNDLED_SYMBOLS` unset when the binary loads extensions. Source: https://github.com/LadybugDB/ladybug-rust/blob/main/src/lib.rs (docs.rs `lbug`)
- So the Rust bindings can use FTS and vector, but only through runtime-loaded shared extensions. I found no Rust-specific test of FTS/vector in `ladybug-rust`; there is an `extension_tests` cargo feature. **UNVERIFIED** end-to-end.
- FTS: BM25, 27 stemmers plus `none`, and only works on STRING properties of node tables. `CREATE_FTS_INDEX` cannot run in a multi-statement block. Changing stopwords requires a rebuild. The docs do not state whether the index is maintained automatically on insert/update/delete (**UNVERIFIED**). Release notes mention "commit-path index maintenance" and an FTS race with "committing writers" (v0.20.0, v0.21.0), which suggests write-time maintenance. Source: https://docs.ladybugdb.com/extensions/full-text-search
- Vector: HNSW-based `vector` extension (https://docs.ladybugdb.com/extensions/vector). An `llm` extension also exists, which computes embeddings through external APIs.
- Prebuilt extension binaries are built in manylinux for glibc compatibility (release note for v0.15.4.2, PR #366). A musl/Alpine runtime is therefore probably unsupported. **UNVERIFIED**.

### Build requirements

- By default, `build.rs` **downloads a precompiled static `liblbug` archive** from the GitHub release (`liblbug-static-linux-{x86_64,aarch64}-{compat,perf}.tar.gz`; default variant `compat`). If that fails, it falls back to a CMake source build of the C++ library. Source: `build.rs` and the `lib.rs` docs in ladybug-rust.
- Environment variables: `LBUG_BUILD_FROM_SOURCE` / `LBUG_RUST_BUILD_FROM_SOURCE`, `LBUG_SHARED`, `LBUG_LIBRARY_DIR`, `LBUG_INCLUDE_DIR`, `LBUG_PREBUILT_CACHE_DIR`, `LBUG_LINUX_VARIANT`, `LBUG_VERSION`, `LBUG_LOCALIZE_BUNDLED_SYMBOLS`.
- The build therefore needs **network access to github.com during `cargo build`**, unless you set the cache dir or library dir, or build from source.
- The build always needs a C++ compiler, because the `cxx`/`cxx-build` bridge (`lbug_rs.cpp`) is compiled even with the prebuilt lib. `cmake` (build-dependency `cmake = "0.1"`) is needed only for the source fallback. The source build uses Python ≥3.9 for the single-file header (`BUILD_SINGLE_FILE_HEADER`). On Linux, static builds link `libatomic`.
- Build time: **UNVERIFIED**. A source build of a Kùzu-sized C++ codebase is typically tens of minutes; I found no official figure.
- Memory note from the crate docs: `SystemConfig::default()` leaves `buffer_pool_size` at 0, so it is auto-sized from host RAM. The docs give an example of "about 6 GB resident on a 15 GiB host for a 200,000-edge graph". They advise setting `buffer_pool_size` explicitly when the machine is shared. This matters on a small VPS.
- Bulk-loading note from the crate docs: a single-row `MERGE … SET` costs "tens of milliseconds of CPU"; use `COPY FROM` or Arrow for bulk loads.

### Concurrency model

- Per the docs, a process has either **one READ_WRITE `Database` object, or multiple READ_ONLY ones**. A READ_WRITE and another Database on the same path cannot be used concurrently.
- Across processes: only one process may open the database READ_WRITE. Other processes may open it READ_ONLY only if no writer has it open. A file lock enforces this: `IO exception: Could not set lock on file`. Source: https://docs.ladybugdb.com/concurrency/
- Within a process: you can create many `Connection`s from one `Database`, and threads may each use a connection concurrently for reads and writes. Every statement runs in a transaction, and transactions are serializable. Source: same page.
- Rust: `Database` is `unsafe impl Send + Sync`. The doc comment on `Connection` says "Each connection is thread-safe, and multiple connections can connect to the same Database instance in a multithreaded environment." Source: `src/database.rs`, `src/connection.rs`
- A newer `SystemConfig::enable_multi_writes(bool)` option (default false, "If true, multiple concurrent write transactions are allowed") exists in the Rust bindings. The concurrency docs page does not document it.
- Consequence: a separate backup or CLI process cannot open the DB while the server holds it READ_WRITE. Backups must go through the server process, for example with `EXPORT DATABASE`.

### On-disk format stability

- There is no stability guarantee. The format has a storage version that changes between releases: "Update storage version" (v0.13.0), "Storage version compat upgrade" and "Update storage version migration handling" (v0.17.0), and "Fix canRead to accept storage version 46" (v0.20.3). Source: release notes at https://github.com/LadybugDB/ladybug/releases
- The official migration path between versions is `EXPORT DATABASE` → `IMPORT DATABASE` (https://docs.ladybugdb.com/migrate/).
- Kùzu files are not directly usable in LadybugDB, according to a secondary source (oneuptime blog, 2026-08-12).

### Export / import

- `EXPORT DATABASE '/path' (format="parquet"|"csv", header=true, schema_only=…)` writes `schema.cypher`, `macro.cypher`, `copy.cypher` and data files. **Parquet is the default.** `IMPORT DATABASE '/path'` requires an empty DB and does not roll back automatically on failure. Indexes are exported only if their extensions are loaded; missing extensions auto-load on import. Source: https://docs.ladybugdb.com/migrate/
- `COPY … FROM` reads CSV, Parquet and Arrow. v0.21.0 added "Export database in parquet-based CSR format (icebug-disk compatible)".
- Note: the plan says "export as Cypher". The export is Cypher DDL plus `COPY` statements plus CSV/Parquet data, **not** a pure Cypher `CREATE` dump.

---

## 2. Alternatives for an embedded store (status only)

| Option | Latest | Status facts | Source |
| --- | --- | --- | --- |
| `rusqlite` | 0.40.2 (2026-08-08) | With the `bundled` feature, `libsqlite3-sys` compiles SQLite with `-DSQLITE_ENABLE_FTS5`, FTS3 and RTREE. Recursive CTEs are core SQLite. | https://crates.io/crates/rusqlite, `libsqlite3-sys/build.rs` |
| `sqlx` | 0.9.0 (2026-05-21) | The SQLite driver has a `bundled` feature (via `libsqlite3-sys`). | https://crates.io/crates/sqlx |
| `sqlite-vec` | stable 0.1.9; prerelease 0.1.10-alpha.4 (2026-05-18) | 8.2k stars; last push 2026-05-18; still pre-1.0. Rust crate compiles the C extension. | https://github.com/asg017/sqlite-vec |
| `tantivy` | 0.27.0 (2026-10-17) | 16.2k stars, active (Quickwit). | https://crates.io/crates/tantivy |
| CozoDB (`cozo`) | 0.7.6 (2023-12-11) | Datalog, not Cypher. No release since 2023-12; last commit 2024-12-04. Effectively unmaintained. | https://github.com/cozodb/cozo |
| Oxigraph | 0.5.11 (2026-09-02) | RDF/SPARQL graph store in pure Rust, active (2.0k stars). **Not** a property graph and no Cypher. | https://github.com/oxigraph/oxigraph |
| Velr | 0.2.52 (updated 2026-10-14) | Embedded openCypher on top of SQLite, self-described alpha; ~1.2k downloads. | https://crates.io/crates/velr |
| uni-db | 4.1.1 (2026-09-24) | OpenCypher, vector and Lance storage; created 2026-03; ~1.1k downloads. | https://crates.io/crates/uni-db |
| SparrowDB | 0.1.27 (2026-08-17) | Embedded Cypher, created 2026-03; ~800 downloads. | https://crates.io/crates/sparrowdb |
| graphdblite | 0.1.2 (2026-07-09) | Claims full openCypher TCK; 75 downloads. | https://crates.io/crates/graphdblite |

All the Rust-native Cypher engines above are less than a year old and have very low adoption.

---

## 3. `rmcp` (official Rust MCP SDK)

- **Latest: `rmcp` 3.5.1 (2026-10-05)**, `rmcp-macros` 3.5.1. The repo has 4.0k stars and was last pushed 2026-10-17. rmcp 3.0.0 was released 2026-07-28. Source: https://crates.io/crates/rmcp, https://github.com/modelcontextprotocol/rust-sdk
- **Spec revisions**: `ProtocolVersion::LATEST = 2026-07-28`. It also supports 2025-11-25, 2025-06-18, 2025-03-26 and 2024-11-05. `LATEST_WITH_INITIALIZE = 2025-11-25`, because the 2026-07-28 revision removes the `initialize` handshake. Source: `crates/rmcp/src/model.rs`
- **Streamable HTTP server**: enabled with the `transport-streamable-http-server` feature. `StreamableHttpService` is a tower `Service`, mounted in axum with `Router::new().nest_service("/mcp", mcp_service)` (see `examples/servers/src/simple_auth_streamhttp.rs`).
- **Stateless mode**: `StreamableHttpServerConfig { legacy_session_mode, json_response, allowed_hosts, allowed_origins, … }`. Per SEP-2567, the 2026-07-28 revision has no sessions, so those requests are **always served statelessly**. `legacy_session_mode` controls only older protocol versions. This was a breaking change in 3.0: `stateful_mode` was renamed `legacy_session_mode`. `allowed_hosts` defaults to loopback only, so a public deployment must set it, or requests are rejected (DNS-rebinding protection). Source: `streamable_http_server/tower.rs`, `CHANGELOG.md`
- **Auth**: the `auth` feature is **client-side**. It covers the OAuth 2.1 + PKCE flow, RFC 8707, RFC 9728 discovery, RFC 8414/OIDC discovery, DCR, CIMD, scope step-up and token refresh. Separately, `auth-client-credentials-jwt` gives client-credentials support for MCP clients. Source: https://github.com/modelcontextprotocol/rust-sdk/blob/main/docs/OAUTH_SUPPORT.md
- **Server side**: rmcp has **no built-in resource-server middleware**. It does not do JWT/JWKS validation and does not serve Protected Resource Metadata. The examples implement auth as an axum `middleware::from_fn` that checks `Authorization: Bearer …` (`simple_auth_streamhttp.rs`). `complex_auth_streamhttp.rs` and `cimd_auth_streamhttp.rs` hand-roll `/.well-known/oauth-protected-resource` and the authorization-server endpoints. You must implement PRM, the 401 + `WWW-Authenticate` response and audience validation yourself.
- **Resources and prompts**: `ServerHandler` has `list_resources`, `list_resource_templates`, `read_resource`, `subscribe`, `list_prompts`, `get_prompt` and `complete`. The macros include `#[tool]`, `#[tool_router]`, `#[tool_handler]`, `#[prompt]`, `#[prompt_router]` and `#[prompt_handler]`. Source: `crates/rmcp/src/handler/server.rs`, `crates/rmcp-macros/src/lib.rs`

---

## 4. MCP authorization spec (latest revision **2026-07-28**)

Source: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization (plus its subpages `authorization-server-discovery` and `client-registration`). Revisions listed in llms.txt: 2024-11-05, 2025-03-26, 2025-06-18, 2025-11-25, 2026-07-28, and draft.

- Authorization is **OPTIONAL**. When used over HTTP, implementations SHOULD conform.
- The MCP server is an OAuth 2.1 **resource server**.
  - It **MUST implement RFC 9728 Protected Resource Metadata**, and the PRM **MUST** contain `authorization_servers`.
  - It MUST provide either the `resource_metadata` parameter in `WWW-Authenticate` on 401, or a well-known URI. The well-known URI is `/.well-known/oauth-protected-resource/<mcp-path>`, or `/.well-known/oauth-protected-resource` at the root.
  - It SHOULD include `scope` in the `WWW-Authenticate` header.
  - It **MUST validate the token audience** (tokens issued specifically for it, RFC 8707 §2). It MUST NOT accept or pass through other tokens.
  - It returns 401 for an invalid or expired token, and SHOULD return 403 with `error="insufficient_scope"`.
  - Tokens arrive only in the `Authorization: Bearer` header, never in the query string.
- The authorization server MUST offer RFC 8414 or OIDC Discovery. Clients MUST try both, in a defined order, including path-insertion variants.
- Client registration, in priority order:
  1. Pre-registered client.
  2. **Client ID Metadata Documents (CIMD)**: SHOULD. Used if the AS advertises `client_id_metadata_document_supported: true`.
  3. **Dynamic Client Registration (RFC 7591)**: MAY, and **deprecated** ("retained for backwards compatibility").
  4. Prompt the user.
- PKCE: the flow mandates S256 (`code_challenge`).
- RFC 9207: the `iss` parameter is validated by the client.
- **RFC 8707 `resource`**: clients MUST send it in both the authorization and token requests, using the canonical MCP server URI (for example `https://mcp.example.com/mcp`).
- **Client credentials** are **not** in the core spec. They are defined in an optional extension, "OAuth Client Credentials Extension", which is still in **draft** in https://github.com/modelcontextprotocol/ext-auth (`specification/draft/oauth-client-credentials.mdx`). It requires pre-registered credentials (no DCR) and recommends JWT client auth (RFC 7523). The core spec mentions `client_credentials` clients only in the step-up section.
- The 2026-07-28 base protocol also removes sessions and `initialize` (see §3).

---

## 5. Client connection and authentication

### Claude.ai / Claude Desktop / mobile / Cowork (custom connectors)

Sources: https://claude.com/docs/connectors/building/authentication and https://claude.com/docs/connectors/custom/add-unlisted

- Remote MCP connections are made **from Anthropic's cloud**, not from the user's machine. Egress range: **`160.79.104.0/21`** (https://platform.claude.com/docs/en/api/ip-addresses). The **authorization server's discovery endpoints must also be reachable from that range**.
- A custom connector by URL is available on Free (one connector), Pro, Max, Team and Enterprise. On Team and Enterprise, the Owner adds it. There are also "MCP tunnels" for servers on private networks.
- Supported auth types:
  - `oauth_dcr`
  - `oauth_cimd`
  - `none`
  - "Use your own OAuth client": a pre-registered client ID, with an **optional** secret, entered in the dialog. Redirect URI: `https://claude.ai/api/mcp/auth_callback`.
  - `static_headers`
- `static_headers` means a static API key or bearer token sent in the `Authorization`, `x-api-key` or `x-auth-token` header. It is "**in beta and available to a limited set of organizations**". If the dialog has no "Request headers" section, the feature is not available to you.
- **"A machine-to-machine `client_credentials` grant isn't supported."** The flow is always the authorization code flow with a user in the loop.
- Claude-specific requirements:
  - A 401 is required to start sign-in.
  - `resource` in the PRM must equal the URL exactly as the user enters it.
  - Only the first `authorization_servers` entry is used.
  - **CIMD is used only if the AS metadata advertises both `client_id_metadata_document_supported: true` and `"none"` in `token_endpoint_auth_methods_supported`.** Otherwise Claude falls back to DCR.
  - S256 PKCE is required, and the AS should advertise `code_challenge_methods_supported`.
  - Timeouts: 10 s for discovery, registration and token; 30 s for refresh.
  - `offline_access` is appended if the AS advertises it.
  - Refresh tokens for public clients should rotate.
  - Return `invalid_grant` on a dead refresh token.

### Claude Code

Source: https://code.claude.com/docs/en/mcp

- `claude mcp add --transport http <name> <url> --header "Authorization: Bearer …"` is supported, and so is `.mcp.json` `headers`, with `${VAR}` expansion.
- `headersHelper`: a script that outputs JSON headers. It runs before each connection and is re-run on a 401 or 403. It can be used to fetch short-lived tokens, for example by running a client-credentials grant **outside** Claude Code.
- OAuth:
  - Uses the authorization code flow with a loopback redirect (`http://localhost:<port>/callback` and `127.0.0.1`), via DCR, CIMD (Claude Code has its own CIMD document), or pre-configured `--client-id`, `--client-secret` (or `MCP_CLIENT_SECRET`) and `--callback-port`.
  - `oauth.scopes` and `oauth.authServerMetadataUrl` are configurable.
  - `claude mcp login <name> [--no-browser]`.
- I found no `client_credentials` grant support in the docs (a grep of the full page found none).

### Mistral Vibe (CLI) and Le Chat

- **Vibe CLI** (https://github.com/mistralai/mistral-vibe, v2.26.1, 2026-10-09; README "MCP Server Configuration"):
  - Config: TOML `[[mcp_servers]]` in the Vibe config, with `transport = "stdio" | "http" | "streamable-http"`.
  - Auth block:
    - `[mcp_servers.auth] type = "static"` takes `headers = {…}`, `api_key_env`, `api_key_header` and `api_key_format = "Bearer {token}"`.
    - `type = "oauth"` takes `scopes`, plus optional `client_id`, `client_metadata_url` and `redirect_port`.
  - CLI: `vibe mcp add <name> --url … --transport streamable-http --api-key-env VAR` or `--header`. Without static options it uses OAuth with a browser login.
  - The OAuth implementation (`vibe/core/auth/mcp_oauth.py`) uses the official MCP Python SDK `OAuthClientProvider`:
    - authorization code + PKCE, loopback `http://127.0.0.1:<redirect_port>/callback`;
    - `grant_types = ["authorization_code","refresh_token"]` and `token_endpoint_auth_method="none"` (a public client);
    - DCR, optional CIMD (`client_metadata_url`) or an optional pre-registered public `client_id`.
  - **No client-credentials grant.**
- **Le Chat custom connectors**: the Mistral docs page is now titled under "Vibe Work". Source: https://docs.mistral.ai/le-chat/knowledge-integrations/connectors/mcp-connectors
  - Transport: Streamable HTTP.
  - Auth is auto-detected and is one of: none; **HTTP Bearer token / Basic auth** (the `Authorization` header); **OAuth 2.1 with dynamic client registration**.
  - Admin-only; on Free, Pro and Student plans the account owner is the admin.
  - Not supported yet: resources and prompt templates.
  - Egress IPs: not documented. Pre-registered OAuth client option: not documented (**UNVERIFIED**).

### Client-credentials summary

None of Claude.ai/Desktop, Claude Code, Vibe CLI or Le Chat performs an OAuth `client_credentials` grant itself. Claude.ai says so explicitly. Static bearer headers are available in Claude Code, Vibe CLI and Le Chat, and in Claude.ai only in a limited beta. In Claude Code, `headersHelper` can wrap an external client-credentials fetch.

---

## 6. Rauthy (sebadob/rauthy)

- **Latest: v0.37.1 (2026-10-07)**. v0.37.0 (2026-10-03) was a breaking release: config renames, and access tokens now carry the JWT header `typ: at+jwt` (RFC 9068). Resource servers that check for `typ == "JWT"` will break. The repo has 1.4k stars and was last pushed 2026-10-15. Source: https://github.com/sebadob/rauthy/releases
- **Dynamic Client Registration**: supported ("OpenID Connect Dynamic Client Registration").
  - Config `[dynamic_clients] enable = false` by default (env `ENABLE_DYN_CLIENT_REG`).
  - Optional `reg_token`, which requires `Authorization: Bearer <DYN_CLIENT_REG_TOKEN>` on registration. A public MCP client like Claude cannot supply it, so open DCR would be needed.
  - Dynamic clients cannot declare `allowed_resources`. The operator must set `[dynamic_clients] allowed_resources` (env `DYN_CLIENT_ALLOWED_RESOURCES`), or the `resource` request fails with `invalid_target`.
  - Source: `book/src/config/config.md`
- **CIMD**: supported as "Ephemeral clients" (`[ephemeral_clients] enable = false` by default). v0.37.1 hardened the SSRF protection of the CIMD fetcher and added configurable `default_scopes`. The well-known metadata has a `client_id_metadata_document_supported` field.
  - **However**, `token_endpoint_auth_methods_supported` in the well-known metadata is hardcoded to `["client_secret_post", "client_secret_basic"]` (`src/data/src/entity/well_known.rs`, main, 2026-10-18). It does not include `"none"`.
  - Per Claude's rules, Claude.ai would therefore **not choose CIMD with Rauthy and would fall back to DCR**. This is inferred from source; **not tested**.
- **Client credentials grant**: supported. `grant_types_supported` includes `client_credentials`, and `resource` is accepted on `client_credentials` token requests.
- **RFC 8707 resource indicators**:
  - Supported on `/oidc/authorize` and on `/oidc/token` for the `authorization_code`, `client_credentials` and `refresh_token` grants, and on token exchange.
  - Each client has an allow-list in `allowed_resources` (deny by default, `invalid_target`) and a `default_aud`. The access token `aud` is the client_id, plus `default_aud`, plus the granted resource.
  - Only a single `resource` per request.
  - Not processed for the password or device-code grants. Per v0.37.0 it is now carried through upstream-provider logins ("fixes e.g. Claude's MCP connector in combination with Upstream Auth Providers").
  - Source: `book/src/work/resource_indicators.md`
- **JWKS**: yes, with automatic key rotation. New clients default to `ed25519` (EdDSA) signing and S256 PKCE. A resource server's JWT library must support EdDSA, or the client must be switched to RS256 or another algorithm.
- **RAM**: according to the README, with allocator tuning and few users it uses about 40 MB (Hiqlite single instance), about 65 MB (Hiqlite HA) or about 35 MB (Postgres). SwaggerUI or the IPGeo DB add more. Argon2id password hashing memory is configurable, and logins temporarily need more.

---

## 7. Markdown: `comrak`

- **Latest: 0.56.0 (2026-10-06)**; 1.7k stars, active. Source: https://crates.io/crates/comrak
- **Wikilinks** have two separate options:
  - `extension.wikilinks_title_after_pipe` parses `[[url|link label]]`, so the **target comes first and the label after the pipe**.
  - `extension.wikilinks_title_before_pipe` parses `[[link label|url]]`.
  - If both are enabled, `title_after_pipe` wins. Output is `<a href="url" data-wikilink="true">label</a>`, and the AST has a `NodeValue::WikiLink` node.
  - Source: `src/parser/options.rs`
- In comrak the second part of `[[Entity|relation]]` is the **display label**. Using it as a relation type would mean post-processing the AST yourself.
- **Alerts**: `extension.alerts` supports **only the 5 GitHub types**: `note`, `tip`, `important`, `warning` and `caution` (`src/scanners.re`, `AlertType`). `> [!decision]` is **not** recognised and would render as a plain blockquote. Custom callout types would need your own processing.
- **Front matter**: `extension.front_matter_delimiter = Some("---")`. comrak only separates the front matter as raw text (a `FrontMatter` node) and does **not** parse YAML, so you need a separate YAML crate.
- Also available: `footnotes`, `inline_footnotes`, `tasklist`, `tasklist_in_table`, `table`, `strikethrough`, `autolink`, `math_*`, `description_lists`, `multiline_block_quotes`, `link_url_rewriter`, `broken_link_callback` and `alert_style`.
- Mermaid is not rendered by comrak; it is just a fenced code block.

---

## 8. askama / htmx / axum

- **askama 0.16.1 (2026-09-04)**. Framework integration crates such as `askama_axum` were removed in askama 0.13 (`askama_axum` 0.5.0+deprecated). The current add-on is **`askama_web` 0.16.0** (2026-04-29), "compatibility add-on… many different web frameworks". askama has no htmx-specific integration; it is not needed.
- **`axum-htmx` 0.8.1** (2025-06-05) provides htmx extractors, responders and guards for axum 0.8.
- **axum 0.8.9 (2026-04-14)** is the latest; no 0.9 has been published on crates.io.
- Related: `jsonwebtoken` 11.1.0 (2026-09-16); `openidconnect` 4.0.1 (2025-07-06).

---

## Points that bear on the current plan

1. **Agents using client-credentials tokens (plan §2, §6)** does not match how the target clients work. Claude.ai explicitly does not support `client_credentials`. Claude Code and Vibe do not implement it either. Client credentials are only a draft MCP extension. What the clients actually support:
   - the OAuth authorization code flow (with DCR, CIMD or a pre-registered client);
   - static bearer headers: in Claude Code, Vibe CLI and Le Chat, and in Claude.ai only in a limited beta.
2. **The Claude custom connector requires** PRM (RFC 9728), a 401 with `resource_metadata`, an AS reachable from `160.79.104.0/21`, S256 PKCE and audience validation (RFC 8707).
   - With Rauthy that means open DCR plus `allowed_resources`, or the pre-registered "use your own OAuth client" option with redirect `https://claude.ai/api/mcp/auth_callback`.
   - CIMD likely will not be selected, because Rauthy does not advertise `none`.
   - **IP allowlisting of the MCP endpoint** must allow Anthropic's range, and the same applies to Rauthy's discovery endpoints.
3. **LadybugDB FTS/vector in Rust** work only as runtime-loaded extensions: download from `extension.ladybugdb.com` or self-host, plus `-rdynamic`. They are not bundled. "No second search infrastructure" still holds, but the Docker build must vendor the `.lbug_extension` files and `LOAD` them on each connection.
4. **The LadybugDB on-disk format changes between versions**. Upgrades mean export and re-import. Only one READ_WRITE process can open the DB, so backups must run in-process. Memory is auto-sized from host RAM unless `buffer_pool_size` is set, which matters on a CX22.
5. **`[[Entity|relation]]`** conflicts with comrak's wikilink semantics, where the second part is a label. **`> [!decision]`** is not a comrak alert type.
6. **rmcp** gives no server-side OAuth/PRM helpers: JWT validation, PRM and the 401 challenge must be written in axum middleware. The newest spec revision (2026-07-28) is stateless, so the server should set `allowed_hosts` for its public hostname.
7. **Rauthy 0.37** issues `at+jwt` access tokens signed with EdDSA by default. The JWT validation code must accept both.
