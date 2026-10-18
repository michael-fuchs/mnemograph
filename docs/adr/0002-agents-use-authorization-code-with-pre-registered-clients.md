# Agents use authorization code with pre-registered clients

The initial plan gave each agent an OAuth client-credentials client. No target MCP client (Claude Code, Claude.ai, Vibe, Le Chat) performs that grant: they use the authorization code flow with PKCE. So each agent installation gets its own **pre-registered** Rauthy client, and the owner logs in once in the browser to authorize it. Dynamic client registration stays off, so no unknown client can register itself. A separate client per installation still allows revoking agents individually and gives every revision and note a trustworthy author. Mnemograph acts as the OAuth resource server: it validates tokens locally against Rauthy's JWKS and serves RFC 9728 Protected Resource Metadata itself, since `rmcp` has no server-side auth.

## Consequences

- Rauthy also runs locally (docker-compose in this repo) so that development uses the real flow; `../server-infra` owns the production instance.
- Claude.ai custom connectors (later) need either a pre-registered client with Claude's callback URL or open DCR, plus an authorization server reachable from Anthropic's egress range.
