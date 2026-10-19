# Rauthy as local OAuth AS for mnemograph + Claude Code (research, 2026-10-18)

Facts for running Rauthy 0.37.x locally (Docker, Hiqlite, plain `http://localhost`) as the
authorization server for mnemograph, with Claude Code as a pre-registered public client.
These are facts, not decisions. Builds on `2026-10-initial-stack-facts.md` §4–6.

**How verified.** Unless marked otherwise, every item was checked in one or both of these ways:

- **[src]**: read in the Rauthy source at tag `v0.37.1` (shallow clone of `sebadob/rauthy`).
  Paths below are relative to that repo.
- **[live]**: tested against `ghcr.io/sebadob/rauthy:0.37.1` running in Docker on this machine.
  The test ran the complete authorization-code + PKCE + `resource` flow through a headless
  Chromium against a random loopback port, then the code exchange, a refresh and a
  refresh-token reuse.

Claude Code facts come from:

- https://code.claude.com/docs/en/mcp (fetched 2026-10-18);
- the installed Claude Code **2.1.292** bundle (string inspection);
- the MCP TypeScript SDK (`modelcontextprotocol/typescript-sdk`, main @ `9bc40e9`, 2026-10-13).

Items marked **UNVERIFIED** were not confirmed.

---

## 1. Running Rauthy 0.37.1 locally (Hiqlite, plain HTTP)

- **Image**: `ghcr.io/sebadob/rauthy:0.37.1`.
  - The base is distroless and the container runs as `10001:10001`.
  - `WORKDIR /app`, `ENTRYPOINT ["/app/rauthy"]`, `CMD ["serve"]`.
  - The image only ships `config-local-test.toml`, `data/` and `tls/`.
  - Source: `Dockerfile`.
- **Config file**: `serve` reads `-c ./config.toml` by default, i.e. **`/app/config.toml`** [src: `src/bin/src/cli_args.rs`].
  - A missing file only logs a warning, and Rauthy continues on defaults plus env vars [src: `RauthyConfig::load`].
  - So env-only works in principle, but a mounted TOML is the documented way.
  - Env vars override TOML values. Each key's env name is in the reference config, `config.toml` in the repo root.
- **`LOCAL_TEST=true`** (the book's quick start) loads the baked-in `config-local-test.toml`. Don't use it here:
  - it serves **https on 8443 with a self-signed cert**;
  - it enables DCR, ephemeral clients and user registration.
- **DB**: Hiqlite is the default (`[database] hiqlite = true`), with data in `data/`, i.e. `/app/data`. Mount a volume there.
- **SMTP**: optional. Without `smtp_url`, Rauthy logs `SMTP_URL is not configured ... cannot send out any E-Mails!` and runs fine [live].
- **Issuer** is derived as `{http|https}://{server.pub_url}/auth/v1/`, **with a trailing slash** [src: `rauthy_config.rs:124`].
  - The scheme is `https` if `server.scheme` contains https **or** `proxy_mode = true`, and `http` otherwise.
  - With the config below: `iss = "http://localhost:8080/auth/v1/"` [live].
- **Cookies on http**:
  - The default `[access] cookie_mode = "host"` sets `__Host-` + `Secure` cookies.
  - For plain-http localhost, set `cookie_mode = "danger-insecure"` (env `COOKIE_MODE`) [src: `src/data/src/api_cookie.rs`].
  - Login over http worked with this setting [live].
  - `[dev] insecure_cookie` (`HQL_INSECURE_COOKIE`) is unrelated: it is only for the Hiqlite dashboard.
- **Loopback redirect ports**: `[access] rfc_8252_enable = true` (env `RFC_8252_ENABLE`). See §3.

### Minimal working `config.toml` (tested [live])

```toml
[bootstrap]
admin_email = "admin@localhost"
password_plain = "DevAdminPassword123!"   # dev only; or password_argon2id
bootstrap_dir = "bootstrap"                # relative to /app -> /app/bootstrap

[cluster]
node_id = 1
nodes = ["1 localhost:8100 localhost:8200"]
secret_raft = "dev-raft-secret-0123456789" # >= 16 chars
secret_api  = "dev-api-secret-0123456789"

[encryption]
# generate: echo "$(openssl rand -hex 4)/$(openssl rand -base64 32)"
keys = ["<keyid>/<base64-32-bytes>"]
key_active = "<keyid>"

[access]
cookie_mode = "danger-insecure"   # plain http on localhost
rfc_8252_enable = true            # loopback redirect URIs match any port
disable_refresh_token_nbf = true  # see §5 "refresh token nbf"

[server]
scheme = "http"
port_http = 8080
pub_url = "localhost:8080"        # host[:port], no scheme
proxy_mode = false

[webauthn]
rp_id = "localhost"
rp_origin = "http://localhost:8080"
```

### Minimal `docker-compose.yml` service (tested [live])

```yaml
services:
  rauthy:
    image: ghcr.io/sebadob/rauthy:0.37.1
    ports: ["8080:8080"]
    volumes:
      - ./rauthy/config.toml:/app/config.toml:ro
      - ./rauthy/bootstrap:/app/bootstrap:ro
      - rauthy-data:/app/data
volumes:
  rauthy-data:
```

- First start log: `Initializing empty production database` → `Migrated 1 API keys.` → listening on `0.0.0.0:8080`.
- The startup takes about 5 s, most of it JWK generation.
- Bind-mounted files must be readable by uid 10001. With `:ro` and world-readable files this works.

---

## 2. Bootstrapping without clicking

Sources: `book/src/config/bootstrap.md`, `bootstrap/*.json`, `src/data/src/migration/bootstrap/types.rs`.

Bootstrap runs **only when the database is empty** (first start). Changing the files later does nothing.

- **Admin user**:
  - Set `[bootstrap] admin_email` (`BOOTSTRAP_ADMIN_EMAIL`).
  - Set the password with `password_plain` (`BOOTSTRAP_ADMIN_PASSWORD_PLAIN`) or `password_argon2id` (`BOOTSTRAP_ADMIN_PASSWORD_ARGON2ID`, which wins if both are set).
  - Without either, a random password is printed once in the log.
  - The admin holds the `rauthy_admin` role.
- **API key**: the preferred route is `api_keys.json` in `bootstrap_dir`. The legacy single key is `[bootstrap] api_key` (a base64 JSON) plus `api_key_secret`.
- **`secret`** takes one of three forms:
  - `{"Plain": "<>= 64 chars>"}`;
  - `{"Encrypted": "<cryptr b64>"}`;
  - `"generate"`. The generated token goes to an encrypted container and is read back with `rauthy bootstrap get --kind api-key --id <name> --field token`. That container is purged after `generated_secrets_ttl`, default 10 min.
- **Hashing**: none needed. Rauthy hashes the secret itself.
- **Access groups**: `Blacklist, Clients, Events, Generic, Groups, Roles, Secrets, Sessions, Scopes, UserAttributes, Users, Pam`.
- **Rights**: `read, create, update, delete`. These map to GET, POST, PUT and DELETE.
- **Header format**: `Authorization: API-Key <name>$<secret>`.
- An API key cannot manage other API keys. API keys can only be created in the Admin UI or by bootstrap.

Tested `bootstrap/api_keys.json` [live]:

```json
[
  {
    "name": "bootstrap",
    "exp": 4102444800,
    "secret": { "Plain": "<64+ chars, e.g. openssl rand -hex 32>" },
    "access": [
      { "group": "Clients", "access_rights": ["read", "create", "update", "delete"] },
      { "group": "Scopes",  "access_rights": ["read", "create", "update", "delete"] }
    ]
  }
]
```

- **Needed rights**:
  - creating and updating clients needs `Clients: create, update`;
  - creating scopes needs `Scopes: create`;
  - reading or rotating a confidential client's secret also needs `Secrets: read`.
- The book recommends a short `exp` for production. That doesn't matter for local dev.

**Alternative: bootstrap clients and scopes declaratively.** Rauthy also reads `scopes.json`, `clients.json`, `users.json`, `roles.json`, `groups.json` and `user_attributes.json` from `bootstrap_dir`.

- The bootstrap `Client` struct has **no `allowed_resources` / `default_aud`** fields [src: `types.rs`].
- So RFC 8707 setup still needs one `PUT /auth/v1/clients/{id}` afterwards.
- Alternatively, rely on `default_aud`… which also can't be bootstrapped.

Example `scopes.json`: `[{"name": "mnemograph"}]`.

---

## 3. Admin API: creating the Claude Code client

Base: `http://localhost:8080/auth/v1`. Sources: `src/api/src/clients.rs`, `src/api_types/src/clients.rs`, `src/service/src/client.rs`.

Creating a client takes **two calls**:

1. **`POST /auth/v1/clients`** takes a `NewClientRequest`, which accepts **only** `id`, `name`, `confidential`, `redirect_uris` and `post_logout_redirect_uris`.
   - Defaults applied [src `impl Default for Client`, confirmed live]:
     - `flows_enabled: ["authorization_code"]`;
     - `access_token_alg`/`id_token_alg: "EdDSA"`;
     - `auth_code_lifetime: 60`, `access_token_lifetime: 1800`;
     - `scopes: ["openid","email","profile","groups"]`, `default_scopes: ["openid"]`;
     - `challenges: ["S256"]`.
   - The `id` regex is `^[a-zA-Z0-9._\-]{2,256}$`.
2. **`PUT /auth/v1/clients/{id}`** takes a full `UpdateClientRequest`, which **replaces everything**.
   - **Pitfall:** omitting `challenges` sets PKCE enforcement to `None` [src: `client.challenge = client_req.challenges.map(..)`].
   - Always send `"challenges": ["S256"]`.

```bash
K='Authorization: API-Key bootstrap$<secret>'
curl -X POST http://localhost:8080/auth/v1/scopes -H "$K" -H 'Content-Type: application/json' \
  -d '{"scope":"mnemograph"}'
curl -X POST http://localhost:8080/auth/v1/clients -H "$K" -H 'Content-Type: application/json' \
  -d '{"id":"claude-code","name":"Claude Code","confidential":false,"redirect_uris":["http://localhost/callback"]}'
curl -X PUT http://localhost:8080/auth/v1/clients/claude-code -H "$K" -H 'Content-Type: application/json' -d '{
  "name": "Claude Code",
  "confidential": false,
  "enabled": true,
  "redirect_uris": ["http://localhost/callback", "http://127.0.0.1/callback"],
  "flows_enabled": ["authorization_code", "refresh_token"],
  "access_token_alg": "EdDSA",
  "id_token_alg": "EdDSA",
  "auth_code_lifetime": 60,
  "access_token_lifetime": 1800,
  "scopes": ["openid", "profile", "email", "groups", "mnemograph"],
  "default_scopes": ["openid", "mnemograph"],
  "challenges": ["S256"],
  "force_mfa": false,
  "allowed_resources": ["http://localhost:3000/mcp"],
  "default_aud": []
}'
```

All three calls returned 200 [live].

- **Required fields of `UpdateClientRequest`**:
  - `confidential`, `redirect_uris`, `enabled`, `flows_enabled` (non-empty);
  - `access_token_alg`, `id_token_alg`;
  - `auth_code_lifetime` (10–300) and `access_token_lifetime` (10–86400);
  - `scopes`, `default_scopes`, `force_mfa`.
- **Optional fields**:
  - `name`, `post_logout_redirect_uris`, `allowed_origins`, `challenges`;
  - `client_uri`, `contacts`, `backchannel_logout_uri`, `restrict_group_prefix`;
  - `claims`, `claims_at_root`, `allowed_resources`, `default_aud`, `scim`.
- **Enum values**:
  - `flows_enabled`: `authorization_code`, `client_credentials`, `password`, `refresh_token`, `urn:ietf:params:oauth:grant-type:device_code`, `urn:ietf:params:oauth:grant-type:token-exchange`;
  - algorithms: `RS256 | RS384 | RS512 | EdDSA | Ed25519`.
- **Public client**: `confidential: false` means no secret. The client authenticates with `client_id` in the body plus `code_verifier` [live].
- **The token endpoint metadata** (`token_endpoint_auth_methods_supported`) still lists only `client_secret_post`/`client_secret_basic`. This doesn't block a pre-registered public client: the code exchange without a secret worked [live].

### Redirect URI matching [src: `Client::validate_redirect_uri_with`, clients.rs:1399; tests at ~3033; live]

- **Scheme and host**:
  - Scheme and host must match exactly.
  - `localhost` and `127.0.0.1` are **different hosts**, so register both if both may be used.
  - A query is part of the match, and `#` or `,` are rejected.
- **Port**:
  - With `rfc_8252_enable = false` (the default), the port must match exactly.
  - With `rfc_8252_enable = true` and a loopback host (`localhost`, `127.0.0.1`, `[::1]`), **the port is ignored**.
  - So registering `http://localhost/callback` accepts `http://localhost:<any>/callback`.
  - Live: `http://localhost:54321/callback` and `http://127.0.0.1:54321/callback` both passed, and a wrong path (`/cb`) gave 400.
  - The reference config's comment says this "only applies to dynamic and ephemeral clients". **The 0.37.1 code does not check the client type**: it applies to static clients too (unit test with static id `loopback_wc`, and live).
- **Path wildcard**: a trailing `*` is a path prefix wildcard, e.g. `http://127.0.0.1/*`.
- **Port wildcard**: `http://localhost:*/callback` is **not** valid. `Url::parse` fails, so there is no port wildcard syntax. Use `rfc_8252_enable` instead.
- **Without `rfc_8252_enable`**: the alternative is a fixed `--callback-port` in Claude Code with `http://localhost:<port>/callback` registered.

---

## 4. Custom scope `mnemograph`

- **Create a scope**: `POST /auth/v1/scopes` with `{"scope": "mnemograph"}`, needing the `Scopes: create` right.
  - Optional fields: `attr_include_access`, `attr_include_id` and `claims_at_root`, which map custom user attributes into tokens.
  - The response is `{"id": "...", "name": "mnemograph", ...}` [live].
- **Discovery**: new scopes appear in `scopes_supported` (it is rebuilt from the DB).
- **Allowing a scope for a client**: the client's `scopes` (allowed) and `default_scopes` (always added).
- **Requested scopes** [src: `sanitize_login_scopes_with`; live]:
  - Granted scopes = `default_scopes` + those requested scopes that are in `scopes`.
  - **Unknown or unallowed requested scopes are silently dropped**, not rejected. Live: `bogus` was dropped.
- **The access token `scope` claim** is a **space-separated string**, e.g. `"openid mnemograph email groups"` [live].
  - The token endpoint response also has `scope`.
- **Other grants**: only `authorization_code` honors requested scopes. For other grants only `default_scopes` apply (book `custom_scopes_attributes.md`; live with the password grant).
- **Recommendation implied by these facts**: put `mnemograph` in `default_scopes` so the token carries it regardless of what Claude Code requests.

---

## 5. Access token contents (0.37.1)

Decoded from a live auth-code token. The request had `scope=openid email groups mnemograph bogus` and `resource=http://localhost:3000/mcp`:

```json
// header
{"alg": "EdDSA", "kid": "fLyl2zliTCbM9ucxPTTRBm59", "typ": "at+jwt"}
// payload
{
  "iat": 1792331084, "nbf": 1792331084, "exp": 1792332884,
  "iss": "http://localhost:8080/auth/v1/",
  "jti": "HjlOHdJWOQFG",
  "aud": ["claude-code", "http://localhost:3000/mcp"],
  "sub": "Lp8AoocGGIhfKoWZIt4NqVA0",
  "typ": "Bearer",
  "azp": "claude-code",
  "scope": "openid mnemograph email groups",
  "email": "admin@localhost", "email_verified": true,
  "roles": ["rauthy_admin", "admin"],
  "groups": ["admin"]
}
```

Source for the claim set: `src/jwt/src/claims.rs` (`JwtAccessClaims`) and `src/service/src/token_set.rs::build_access_token`.

- **Header**:
  - `typ` is **`at+jwt`** (RFC 9068).
  - `alg` follows the client's `access_token_alg`; EdDSA uses an Ed25519 OKP key.
  - `kid` is always present.
  - Rauthy's own validator accepts `JWT` and `at+jwt`.
- **`iss`**: `http://localhost:8080/auth/v1/`, **with the trailing slash**. Compare it exactly.
- **`sub`**: the Rauthy user id (24-char random), not the email.
- **`aud`** is the client id, plus each `default_aud` entry, plus the granted `resource`, de-duplicated.
  - **A single string** when there is one value (live: `"aud": "probe"` without a resource).
  - **An array** when there are two or more.
  - The resource server must accept both shapes and check that `aud` *contains* its resource URL.
- **`azp`**: the client id. **There is no `client_id` claim.**
- **`typ` claim**: Rauthy's own token type: `Bearer`, or `DPoP` for DPoP-bound tokens. Not to be confused with the header `typ`.
- **`scope`**: a space-separated string.
- **Identity claims**:
  - `email` and `email_verified` only if the scope contains `email`;
  - `roles` are **always** present for user tokens;
  - `groups` only if the scope contains `groups`;
  - **no `preferred_username`** in access tokens. It is only in the ID token.
- **Custom attributes**: under `custom`, or at the root with `claims_at_root`.
- **`jti`**: always present. `did`, `cnf` and `act` appear only for the device flow, DPoP and token exchange.
- **JWKS** is at `GET /auth/v1/oidc/certs`, plus `/auth/v1/oidc/certs/{kid}`.
  - It contains RS256, RS384, RS512 (RSA) and EdDSA (OKP, `crv: Ed25519`) keys.
  - Keys auto-rotate monthly (`jwk_autorotate_cron = '0 30 3 1 * * *'`), so refetch on an unknown `kid`.

### RFC 8707 behaviour [src: `validate_resource_request`, `grant_types/authorization_code.rs`; live]

- **Allow-list check**:
  - `resource` is matched **verbatim** against `allowed_resources`.
  - `http://localhost:3000/mcp/` (trailing slash) is **not** `http://localhost:3000/mcp`.
  - An empty `allowed_resources` means any `resource` is rejected with `invalid_target`.
  - Only one `resource` per request.
- **Where the check happens**:
  - The check on the authorization request happens **after login** (`finish_authorize`), not at `GET /oidc/authorize`. A bad resource still shows the login page.
  - Live: the error is shown **inside Rauthy's login UI** ("the requested `resource` is not allowed for this client"). It is **not** a redirect to the client with `error=invalid_target`, so Claude Code would just wait for a callback that never comes.
- **Token request**:
  - `resource` may be omitted, or else must equal the one granted at authorize.
  - A different value gives `invalid_target`.
  - Live: the exchange with an identical `resource` worked.
- **Refresh**:
  - The granted resource is carried inside the refresh token.
  - Live: a refresh without `resource` kept `aud = ["claude-code", "http://localhost:3000/mcp"]` and `scope`.
  - The refresh token **rotates**: reusing the old one gave `404 "Refresh Token not found"`.
- **Upstream-provider logins**: `resource` is preserved as of #1702 (closed 2026-09-16). The book page still says it isn't, so the book is stale here.

### Refresh token `nbf` (important for dev)

- **Default behaviour**:
  - Refresh tokens get `nbf = now + access_token_lifetime - 60s`.
  - Using a refresh token earlier gives `{"error":"JwtToken","message":"Token is not valid yet"}` [live].
  - Per the config docs, early use also **invalidates linked sessions and tokens**.
  - `[access] disable_refresh_token_nbf = true` (`DISABLE_REFRESH_TOKEN_NBF`) sets `nbf = now`. With it, an immediate refresh worked [live].
- **Rauthy issue #1647**: claude.ai does one probe refresh about 1 s after the code exchange. The default `nbf` rejects that probe. The owner considers `disable_refresh_token_nbf = true` the fix.
- **Claude Code**: the docs say it refreshes on 401. Whether it also probes right after the exchange is **UNVERIFIED**.
- **Recommendation implied by these facts**: set `disable_refresh_token_nbf = true` in dev.

---

## 6. Discovery endpoints [src: `src/api/src/oidc.rs`, `src/bin/src/server.rs`; live HTTP codes]

| URL | Result |
|---|---|
| `/.well-known/openid-configuration` | 200 |
| `/auth/v1/.well-known/openid-configuration` | 200 |
| `/.well-known/oauth-authorization-server` | 200 (same body) |
| `/.well-known/oauth-authorization-server/auth/v1` | 200 (RFC 8414 path insertion, #1643) |
| `/.well-known/oauth-authorization-server/auth/v1/` | 200 |
| `/auth/v1/.well-known/oauth-authorization-server` | 200 |
| `/.well-known/openid-configuration/auth/v1` | **301** (OIDC path-insertion variant not served) |
| `/auth/v1/oidc/certs` (JWKS) | 200 |

All variants serve the same JSON. Fields with these values [live]:

- `issuer`: `http://localhost:8080/auth/v1/`.
- `authorization_endpoint`: `…/auth/v1/oidc/authorize`.
- `token_endpoint`: `…/auth/v1/oidc/token`.
- `jwks_uri`: `…/auth/v1/oidc/certs`.
- `introspection_endpoint`, `revocation_endpoint` (`…/oidc/token/revoke`), `userinfo_endpoint` and `end_session_endpoint`.
- `registration_endpoint`: only when DCR is enabled. Absent here.
- `code_challenge_methods_supported`: `["plain", "S256"]`. S256 is advertised. Per-client `challenges` restricts it to S256.
- `response_types_supported`: `["code"]`.
- `grant_types_supported`: all six grants, independent of per-client config.
- `token_endpoint_auth_methods_supported`: `["client_secret_post", "client_secret_basic"]`. **No `"none"`.**
- `authorization_response_iss_parameter_supported`: `true`. The callback carries `&iss=http%3A%2F%2Flocalhost%3A8080%2Fauth%2Fv1%2F` [live].
- `client_id_metadata_document_supported`: **hardcoded `true`**, even when `[ephemeral_clients] enable = false`. In practice this does not matter, because Claude Code uses `--client-id` when given.
- `scopes_supported`: all scopes in the DB, custom ones included.
- **No field advertises RFC 8707 support.** There is no standard metadata field for it, so clients just send `resource`.

**For PRM**:

- Set `"authorization_servers": ["http://localhost:8080/auth/v1/"]`, the exact issuer with trailing slash.
- The MCP TS SDK strips a trailing `/` before path insertion and tries, in order:
  1. `/.well-known/oauth-authorization-server/auth/v1` (200);
  2. `/.well-known/openid-configuration/auth/v1` (301);
  3. `/auth/v1/.well-known/openid-configuration` (200).
- The SDK tolerates a single trailing-slash difference between the PRM entry and the metadata `issuer` (`issuersMatch`).
- Claude Code issue #95270 (open) reports that some Claude Code versions fetch the **bare** `/.well-known/oauth-authorization-server`. Rauthy serves that too, with the same body, so either path works.

---

## 7. Claude Code (docs + 2.1.292 bundle)

- **Add with a pre-registered public client** (docs, "Use pre-configured OAuth credentials"; no `--client-secret` for a public client):

  ```bash
  claude mcp add --transport http --client-id claude-code mnemograph http://localhost:3000/mcp
  # optional fixed port (must then match a registered URI unless rfc_8252_enable=true):
  claude mcp add --transport http --client-id claude-code --callback-port 33418 mnemograph http://localhost:3000/mcp
  ```

  - JSON form: `claude mcp add-json mnemograph '{"type":"http","url":"http://localhost:3000/mcp","oauth":{"clientId":"claude-code","callbackPort":33418}}'`.
  - Optional `oauth.scopes` (a space-separated string) pins the requested scopes.
  - `oauth.authServerMetadataUrl` exists but **must be `https://`**, so it is unusable for http dev.
- **Redirect URI**: `http://localhost:<port>/callback`.
  - Bundle 2.1.292: ``function NUe(n=f){return`http://localhost:${n}/callback`}``.
  - 2.1.229 briefly used `127.0.0.1`, and 2.1.231 reverted it (docs).
  - Issue #91846 (open) asks for a configurable host.
- **Port selection** (bundle):
  1. `MCP_OAUTH_CALLBACK_PORT` env;
  2. the configured `callbackPort`;
  3. a random free port in 49152–65535 (39152–49151 on Windows), up to 100 tries;
  4. the fallback `3118`.
- **Token endpoint**: Claude Code sends credentials to a plain-http token endpoint only on `localhost`, `127.0.0.1` or `::1` (docs). So `http://localhost:8080` is allowed.
- **Discovery** (docs):
  - Claude Code first checks RFC 9728 PRM at `/.well-known/oauth-protected-resource`, or via the `resource_metadata` in the 401 `WWW-Authenticate`.
  - It then follows `authorization_servers` to RFC 8414 / OIDC metadata.
  - A 401 (or 403) marks the server as needing authentication.
- **`resource` parameter**:
  - The bundle's authorization-URL builder does `if(c)s.searchParams.set("resource",c.href)`.
  - In the SDK (`selectResourceURL`), the resource is sent **only when PRM was found**, and its value is **the PRM `resource` field** (after checking it is compatible with the server URL), serialized via `new URL(..).href`.
  - Consequences:
    - Make PRM `resource` exactly the string listed in Rauthy `allowed_resources`.
    - Use a URL with a path, e.g. `http://localhost:3000/mcp`. A bare origin would be serialized with a trailing `/`.
  - Whether Claude Code also sends `resource` on the token request is **UNVERIFIED**. The SDK has a `resource` arg on the token request; Rauthy accepts either way.
- **Scopes** (SDK `determineScope`; Claude Code docs):
  - Priority:
    1. `oauth.scopes` (if pinned);
    2. the `scope` from `WWW-Authenticate`;
    3. PRM `scopes_supported`;
    4. none.
  - Claude Code does **not** request the AS's full `scopes_supported`.
  - `offline_access` is appended if the AS advertises it. Rauthy doesn't, unless a scope with that name is created.
  - Issue #89862 (open): Claude Code's refresh request omits `scope`. Rauthy keeps the original scope on refresh [live], so this is harmless here.
- **Login**: `claude mcp login mnemograph` (`--no-browser` prints the URL), or `/mcp` → Authenticate in a session.
  - `claude mcp logout <name>` clears the stored credentials.
  - `claude mcp get <name>` shows the OAuth config.
  - On a 401 with a stored token, Claude Code refreshes, reconnects and retries once.

---

## 8. Known issues and incompatibilities

- **Fixed in Rauthy ≤ 0.37**:
  - #1643: RFC 8414 path-insertion URL returned 301 (fixed: the routes above).
  - #1645: `resource` dropped on `/oidc/authorize/refresh`, i.e. a returning user with an existing session (fixed).
  - #1702: `resource` dropped on upstream-provider logins (fixed).
  - #1644: claude.ai's CIMD `grant_types` were rejected (fixed).
  - #1646: `client_credentials` tokens had `"sub": null` (fixed; `[access] client_credentials_map_sub`).
- **Still relevant**:
  - #1647 (closed, config workaround): refresh-token `nbf` versus clients that refresh early. Use `disable_refresh_token_nbf = true`.
  - Header `typ: at+jwt` since 0.37.0: a resource-server JWT library must not insist on `typ == "JWT"`.
  - EdDSA by default: the RS JWT library must support Ed25519. Otherwise switch the client to RS256. One reporter in #1647 hit exactly this.
  - Discovery lacks `"none"` in `token_endpoint_auth_methods_supported`. This doesn't matter for a pre-registered public client [live].
  - A disallowed `resource` produces an error page in Rauthy's UI, not an OAuth error redirect, so Claude Code will hang until timeout.
- **Claude Code side**:
  - #95270: bare AS metadata path. Harmless with Rauthy.
  - #91846: callback host fixed to `localhost`. Register `http://localhost/callback`, and also `127.0.0.1` defensively.
  - #96053: VS Code extension ignores `--callback-port`. Harmless with `rfc_8252_enable`.
- **None found**: no Rauthy-specific issue exists in `anthropics/claude-code` (search "rauthy": 0 results).

---

## What to test end-to-end (not yet done with real Claude Code)

1. **Full sign-in.** Run `claude mcp add --transport http --client-id claude-code mnemograph http://localhost:3000/mcp` against a mnemograph stub that does three things:
   - returns 401 with `WWW-Authenticate: Bearer resource_metadata="http://localhost:3000/.well-known/oauth-protected-resource/mcp", scope="mnemograph"`;
   - serves PRM `{"resource":"http://localhost:3000/mcp","authorization_servers":["http://localhost:8080/auth/v1/"],"scopes_supported":["mnemograph"]}`;
   - logs the incoming `Authorization` header.

   Then run `claude mcp login mnemograph`. Confirm three things:
   - the authorize URL contains `resource=http://localhost:3000/mcp` and `scope=mnemograph`;
   - the redirect is `http://localhost:<random>/callback`;
   - the token has `aud ∋ http://localhost:3000/mcp`.
2. **Early refresh.** Find out whether Claude Code refreshes right after the code exchange. Watch the Rauthy logs with `disable_refresh_token_nbf = false`.
3. **Token request.** Find out whether Claude Code sends `resource` on the token request (Rauthy logs, or a proxy).
