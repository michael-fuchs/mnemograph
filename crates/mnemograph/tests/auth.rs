//! End-to-end tests of OAuth: agents need a valid access token, their identity becomes the
//! author of what they write, and the spheres in their token bound what they see.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header};
use mnemograph::{AppConfig, AuthSettings, Authenticator};
use mnemograph_core::{Author, Knowledge};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::{RoleClient, RunningService},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const ISSUER: &str = "http://localhost:8443/auth/v1";
const KEY_ID: &str = "test-key";

struct Keys {
    signing: EncodingKey,
    jwk: Jwk,
}

impl Keys {
    fn generate() -> Self {
        let mut seed = [0u8; 32];
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut seed);
        let key = SigningKey::from_bytes(&seed);
        let der = key.to_pkcs8_der().unwrap();
        let jwk: Jwk = serde_json::from_value(json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "alg": "EdDSA",
            "kid": KEY_ID,
            "x": URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes()),
        }))
        .unwrap();
        Self {
            signing: EncodingKey::from_ed_der(der.as_bytes()),
            jwk,
        }
    }

    fn token(&self, typ: &str, claims: Value) -> String {
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some(typ.to_owned());
        header.kid = Some(KEY_ID.to_owned());
        jsonwebtoken::encode(&header, &claims, &self.signing).unwrap()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

struct TestServer {
    base: String,
    knowledge: Arc<Knowledge>,
    shutdown: CancellationToken,
    _dir: tempfile::TempDir,
}

impl TestServer {
    async fn start(keys: &Keys) -> Self {
        Self::start_with_forbidden(keys, &[]).await
    }

    /// Configures the spheres work and private, and besides them `forbidden`, whose rule keeps
    /// them out of the wiki.
    async fn start_with_forbidden(keys: &Keys, forbidden: &[&str]) -> Self {
        let forbidden: Vec<String> = forbidden.iter().map(|s| s.to_string()).collect();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let settings = AuthSettings {
            issuer: ISSUER.into(),
            resource: format!("{base}/mcp"),
            required_scopes: vec!["mnemograph".into()],
            spheres: mnemograph_core::Spheres::new(["work", "private"]).unwrap(),
            forbidden_spheres: forbidden.clone(),
        };
        let authenticator = Authenticator::with_keys(
            settings,
            HashMap::from([(
                KEY_ID.to_owned(),
                (DecodingKey::from_jwk(&keys.jwk).unwrap(), Algorithm::EdDSA),
            )]),
        );

        let dir = tempfile::tempdir().unwrap();
        let knowledge = Arc::new(
            Knowledge::open(dir.path().join("test.db"))
                .unwrap()
                .with_forbidden_spheres(forbidden),
        );
        let shutdown = CancellationToken::new();
        let app = mnemograph::app(
            knowledge.clone(),
            AppConfig {
                auth: Some(Arc::new(authenticator)),
                spheres: mnemograph_core::Spheres::new(["work", "private"]).unwrap(),
                allowed_hosts: vec!["127.0.0.1".into()],
                limits: Default::default(),
                web_login: None,
                graph_max_nodes: mnemograph::web::DEFAULT_GRAPH_NODES,
                access_log_days: mnemograph::DEFAULT_ACCESS_LOG_DAYS,
                shutdown: shutdown.child_token(),
            },
        );
        tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown.cancelled_owned())
                    .await
                    .unwrap();
            }
        });
        Self {
            base,
            knowledge,
            shutdown,
            _dir: dir,
        }
    }

    fn valid_claims(&self) -> Value {
        json!({
            "iss": ISSUER,
            "aud": format!("{}/mcp", self.base),
            "sub": "owner-user-id",
            "azp": "claude-code-laptop",
            "scope": "openid mnemograph sphere:work",
            "exp": now() + 300,
        })
    }

    /// Connects an MCP client whose token has these scopes.
    async fn client_with_scope(&self, keys: &Keys, scope: &str) -> RunningService<RoleClient, ()> {
        let mut claims = self.valid_claims();
        claims["scope"] = json!(scope);
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", self.base))
                .auth_header(keys.token("at+jwt", claims)),
        );
        ().serve(transport).await.unwrap()
    }

    /// Sends a raw MCP request and returns status, WWW-Authenticate header and body.
    async fn raw_call(&self, token: Option<&str>) -> (u16, Option<String>, String) {
        let mut request = reqwest::Client::new()
            .post(format!("{}/mcp", self.base))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let challenge = response
            .headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap().to_owned());
        (status, challenge, response.text().await.unwrap())
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[tokio::test]
async fn request_without_token_gets_a_challenge_pointing_to_the_metadata() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;

    let (status, challenge, _) = server.raw_call(None).await;
    assert_eq!(status, 401);
    let metadata_url = format!("{}/.well-known/oauth-protected-resource/mcp", server.base);
    assert_eq!(
        challenge.as_deref(),
        Some(format!("Bearer resource_metadata=\"{metadata_url}\", scope=\"mnemograph\"").as_str())
    );

    let metadata: Value = reqwest::get(&metadata_url)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        metadata,
        json!({
            "resource": format!("{}/mcp", server.base),
            "authorization_servers": [ISSUER],
            "scopes_supported": ["mnemograph"],
            "bearer_methods_supported": ["header"],
            "resource_name": "mnemograph",
        })
    );
}

#[tokio::test]
async fn valid_token_is_accepted_and_its_client_becomes_the_author() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let token = keys.token("at+jwt", server.valid_claims());

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", server.base))
            .auth_header(token),
    );
    let client = ().serve(transport).await.unwrap();
    let args = |v: Value| v.as_object().unwrap().clone();
    let written = client
        .call_tool(
            CallToolRequestParams::new("write_page")
                .with_arguments(args(json!({ "title": "Rust", "body": "A language." }))),
        )
        .await
        .unwrap();
    assert_ne!(written.is_error, Some(true), "{written:?}");
    client
        .call_tool(
            CallToolRequestParams::new("add_note")
                .with_arguments(args(json!({ "body": "About [[Rust]]." }))),
        )
        .await
        .unwrap();

    let agent = Some(Author::Agent("claude-code-laptop".into()));
    assert_eq!(
        server
            .knowledge
            .page("Rust", mnemograph_core::Visibility::All)
            .unwrap()
            .unwrap()
            .author,
        agent
    );
    assert_eq!(server.knowledge.notes().unwrap()[0].author, agent);
}

#[tokio::test]
async fn rfc_9068_client_id_claim_is_preferred() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let mut claims = server.valid_claims();
    claims["client_id"] = json!("from-client-id");
    let (status, _, _) = server.raw_call(Some(&keys.token("at+jwt", claims))).await;
    assert_eq!(status, 200);
}

async fn assert_rejected(server: &TestServer, token: &str, reason: &str) {
    let (status, challenge, _) = server.raw_call(Some(token)).await;
    assert_eq!(status, 401, "{reason}");
    let challenge = challenge.unwrap();
    assert!(
        challenge.contains("error=\"invalid_token\""),
        "{reason}: {challenge}"
    );
}

#[tokio::test]
async fn invalid_tokens_are_rejected() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let with = |change: &dyn Fn(&mut Value)| {
        let mut claims = server.valid_claims();
        change(&mut claims);
        keys.token("at+jwt", claims)
    };

    assert_rejected(
        &server,
        &with(&|c| c["exp"] = json!(now() - 120)),
        "expired",
    )
    .await;
    assert_rejected(
        &server,
        &with(&|c| c["aud"] = json!("http://other-service/api")),
        "wrong audience",
    )
    .await;
    assert_rejected(
        &server,
        &with(&|c| c["iss"] = json!("http://evil.example")),
        "wrong issuer",
    )
    .await;
    assert_rejected(
        &server,
        &keys.token("JWT", server.valid_claims()),
        "an ID token is not an access token",
    )
    .await;
    assert_rejected(
        &server,
        &Keys::generate().token("at+jwt", server.valid_claims()),
        "signed by another key",
    )
    .await;
    assert_rejected(&server, "not-a-jwt", "garbage").await;
}

#[tokio::test]
async fn token_without_the_required_scope_is_forbidden() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let mut claims = server.valid_claims();
    claims["scope"] = json!("openid profile");
    let (status, challenge, _) = server.raw_call(Some(&keys.token("at+jwt", claims))).await;
    assert_eq!(status, 403);
    assert!(challenge.unwrap().contains("error=\"insufficient_scope\""));
}

async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

fn structured(result: &CallToolResult) -> &Value {
    assert_ne!(result.is_error, Some(true), "tool failed: {result:?}");
    result.structured_content.as_ref().unwrap()
}

fn error_text(result: &CallToolResult) -> String {
    assert_eq!(
        result.is_error,
        Some(true),
        "expected a tool error: {result:?}"
    );
    serde_json::to_string(&result.content).unwrap()
}

#[tokio::test]
async fn token_without_a_known_sphere_is_forbidden_with_a_clear_error() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    for scope in ["openid mnemograph", "openid mnemograph sphere:clubs"] {
        let mut claims = server.valid_claims();
        claims["scope"] = json!(scope);
        let (status, challenge, body) = server.raw_call(Some(&keys.token("at+jwt", claims))).await;
        assert_eq!(status, 403, "{scope}");
        let challenge = challenge.unwrap();
        assert!(
            challenge.contains("error=\"insufficient_scope\""),
            "{challenge}"
        );
        assert!(challenge.contains("sphere:<name>"), "{challenge}");
        assert!(
            body.contains("no sphere") && body.contains("private, work"),
            "{body}"
        );
    }
}

#[tokio::test]
async fn an_agent_cannot_read_or_change_a_page_of_another_sphere() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let private = server
        .client_with_scope(&keys, "mnemograph sphere:private")
        .await;
    let work = server
        .client_with_scope(&keys, "mnemograph sphere:work")
        .await;

    let written = call(
        &private,
        "write_page",
        json!({ "title": "Holidays", "body": "Secret plans." }),
    )
    .await;
    assert_eq!(structured(&written)["spheres"], json!(["private"]));
    let read = call(&private, "read_page", json!({ "title": "holidays" })).await;
    assert_eq!(structured(&read)["body"], "Secret plans.");

    let read = error_text(&call(&work, "read_page", json!({ "title": "Holidays" })).await);
    assert!(
        read.contains("no page titled") && !read.contains("stub"),
        "{read}"
    );
    let change = error_text(
        &call(
            &work,
            "write_page",
            json!({ "title": "Holidays", "body": "mine", "base_revision": 1 }),
        )
        .await,
    );
    assert!(change.contains("no page titled"), "{change}");
    let create = error_text(
        &call(
            &work,
            "write_page",
            json!({ "title": "Holidays", "body": "mine" }),
        )
        .await,
    );
    assert!(
        create.contains("not available") && !create.contains("Secret"),
        "{create}"
    );

    let page = server
        .knowledge
        .page("Holidays", mnemograph_core::Visibility::All)
        .unwrap()
        .unwrap();
    assert_eq!((page.revision, page.body.as_str()), (1, "Secret plans."));
    // Authors and the access log are unaffected by spheres.
    assert_eq!(
        page.author,
        Some(Author::Agent("claude-code-laptop".into()))
    );
    let access = server
        .knowledge
        .agent_access(mnemograph_core::Visibility::All)
        .unwrap();
    assert_eq!(access[0].reads, 2);
}

#[tokio::test]
async fn a_page_in_two_spheres_is_visible_to_both() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let work = server
        .client_with_scope(&keys, "mnemograph sphere:work")
        .await;
    let private = server
        .client_with_scope(&keys, "mnemograph sphere:private")
        .await;
    let both = server
        .client_with_scope(&keys, "mnemograph sphere:work sphere:private")
        .await;
    let created = call(
        &both,
        "write_page",
        json!({ "title": "Rust", "body": "A language.", "spheres": ["work", "private"] }),
    )
    .await;
    assert_eq!(structured(&created)["spheres"], json!(["private", "work"]));

    for (client, base) in [(&work, 1), (&private, 2)] {
        let read = call(client, "read_page", json!({ "title": "Rust" })).await;
        assert_eq!(structured(&read)["spheres"], json!(["private", "work"]));
        let changed = call(
            client,
            "write_page",
            json!({ "title": "Rust", "body": format!("revision {}", base + 1), "base_revision": base }),
        )
        .await;
        assert_eq!(structured(&changed)["revision"], base + 1);
    }
}

#[tokio::test]
async fn a_registration_with_two_spheres_must_name_one_when_creating() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let both = server
        .client_with_scope(&keys, "mnemograph sphere:work sphere:private")
        .await;

    let unnamed =
        error_text(&call(&both, "write_page", json!({ "title": "Rust", "body": "" })).await);
    assert!(
        unnamed.contains("name the sphere") && unnamed.contains("private, work"),
        "{unnamed}"
    );
    let foreign = error_text(
        &call(
            &both,
            "write_page",
            json!({ "title": "Rust", "body": "", "sphere": "clubs" }),
        )
        .await,
    );
    assert!(
        foreign.contains("\\\"clubs\\\" is not available"),
        "{foreign}"
    );
    let named = call(
        &both,
        "write_page",
        json!({ "title": "Rust", "body": "", "sphere": "private" }),
    )
    .await;
    assert_eq!(structured(&named)["spheres"], json!(["private"]));

    let unnamed = error_text(&call(&both, "add_note", json!({ "body": "About [[Rust]]." })).await);
    assert!(unnamed.contains("name the sphere"), "{unnamed}");
    let note = call(
        &both,
        "add_note",
        json!({ "body": "About [[Rust]].", "sphere": "work" }),
    )
    .await;
    assert_eq!(structured(&note)["spheres"], json!(["work"]));
}

#[tokio::test]
async fn no_tool_shows_pages_or_notes_of_another_sphere() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let private = server
        .client_with_scope(&keys, "mnemograph sphere:private")
        .await;
    let work = server
        .client_with_scope(&keys, "mnemograph sphere:work")
        .await;
    for (client, tool, args) in [
        (
            &private,
            "write_page",
            json!({ "title": "Holidays", "body": "A sea trip. See [[Setup]] and [[Beach]]." }),
        ),
        (
            &private,
            "add_note",
            json!({ "body": "Booked the [[Holidays]] by the sea." }),
        ),
        (
            &work,
            "write_page",
            json!({ "title": "Setup", "body": "The office by the sea, before the [[Holidays]]." }),
        ),
        (
            &work,
            "add_note",
            json!({ "body": "Sea level data for [[Setup]]." }),
        ),
    ] {
        structured(&call(client, tool, args).await);
    }

    let search = call(&work, "search", json!({ "query": "sea" })).await;
    let mut found: Vec<String> = structured(&search)["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| match hit["kind"].as_str().unwrap() {
            "page" => hit["title"].as_str().unwrap().to_owned(),
            _ => hit["note"]["body"].as_str().unwrap().to_owned(),
        })
        .collect();
    found.sort();
    assert_eq!(found, ["Sea level data for [[Setup]].", "Setup"]);

    let setup = call(&work, "read_page", json!({ "title": "Setup" })).await;
    let setup = structured(&setup);
    assert_eq!(
        setup["links"],
        json!([{ "title": "Holidays", "stub": true }])
    );
    assert_eq!(setup["backlinks"], json!([]));
    assert_eq!(setup["notes"].as_array().unwrap().len(), 1);
    assert_eq!(setup["notes"][0]["body"], "Sea level data for [[Setup]].");

    // The hidden page is a stub that reveals only who in this sphere links to it.
    let holidays = error_text(&call(&work, "read_page", json!({ "title": "Holidays" })).await);
    assert!(
        holidays.contains("stub") && holidays.contains("Setup"),
        "{holidays}"
    );
    assert!(
        !holidays.contains("trip") && !holidays.contains("Booked"),
        "{holidays}"
    );

    let related = call(&work, "related", json!({ "title": "Setup", "hops": 3 })).await;
    assert_eq!(
        structured(&related)["related"],
        json!([{ "title": "Holidays", "stub": true, "distance": 1 }])
    );
    let beach = error_text(&call(&work, "related", json!({ "title": "Beach" })).await);
    assert!(beach.contains("no page titled"), "{beach}");

    // The access log records each registration's sphere.
    let spheres = |name: &str| mnemograph_core::Spheres::new([name]).unwrap();
    let read_by = |name: &str| {
        let spheres = spheres(name);
        server
            .knowledge
            .most_read_pages(10, mnemograph_core::Visibility::Within(&spheres))
            .unwrap()
            .into_iter()
            .map(|page| page.title)
            .collect::<Vec<_>>()
    };
    assert_eq!(read_by("work"), ["Setup"]);
    assert!(read_by("private").is_empty());
}

#[tokio::test]
async fn a_sphere_whose_rule_forbids_the_wiki_is_refused() {
    let keys = Keys::generate();
    let server = TestServer::start_with_forbidden(&keys, &["clubs"]).await;

    // A connection made only of forbidden spheres is refused.
    let mut claims = server.valid_claims();
    claims["scope"] = json!("mnemograph sphere:clubs");
    let (status, challenge, body) = server.raw_call(Some(&keys.token("at+jwt", claims))).await;
    assert_eq!(status, 403);
    assert!(
        challenge.unwrap().contains("error=\"insufficient_scope\""),
        "{body}"
    );
    assert!(
        body.contains("clubs") && body.contains("must not enter the wiki"),
        "{body}"
    );

    // With another sphere, the forbidden one is not usable, and naming it says why.
    let mixed = server
        .client_with_scope(&keys, "mnemograph sphere:clubs sphere:work")
        .await;
    for args in [
        json!({ "title": "Board", "body": "", "sphere": "clubs" }),
        json!({ "title": "Board", "body": "", "spheres": ["clubs", "work"] }),
    ] {
        let refused = error_text(&call(&mixed, "write_page", args).await);
        assert!(refused.contains("must not enter the wiki"), "{refused}");
    }
    let refused = error_text(
        &call(
            &mixed,
            "add_note",
            json!({ "body": "Board meeting.", "sphere": "clubs" }),
        )
        .await,
    );
    assert!(refused.contains("must not enter the wiki"), "{refused}");
    let written = call(
        &mixed,
        "write_page",
        json!({ "title": "Board", "body": "" }),
    )
    .await;
    assert_eq!(structured(&written)["spheres"], json!(["work"]));
}

#[tokio::test]
async fn changing_spheres_keeps_those_the_connection_cannot_see() {
    let keys = Keys::generate();
    let server = TestServer::start(&keys).await;
    let both = server
        .client_with_scope(&keys, "mnemograph sphere:work sphere:private")
        .await;
    let private = server
        .client_with_scope(&keys, "mnemograph sphere:private")
        .await;
    structured(
        &call(
            &both,
            "write_page",
            json!({ "title": "Rust", "body": "", "sphere": "work" }),
        )
        .await,
    );
    let shared = call(
        &both,
        "write_page",
        json!({ "title": "Rust", "body": "", "base_revision": 1, "spheres": ["work", "private"] }),
    )
    .await;
    assert_eq!(structured(&shared)["spheres"], json!(["private", "work"]));

    // The private connection cannot see work, so setting only private keeps work.
    let kept = call(
        &private,
        "write_page",
        json!({ "title": "Rust", "body": "Ferris.", "base_revision": 2, "spheres": ["private"] }),
    )
    .await;
    assert_eq!(structured(&kept)["spheres"], json!(["private", "work"]));
    let empty = error_text(
        &call(
            &private,
            "write_page",
            json!({ "title": "Rust", "body": "", "base_revision": 3, "spheres": [] }),
        )
        .await,
    );
    assert!(empty.contains("at least one sphere"), "{empty}");

    // A connection seeing both can move it out of private; then private no longer sees it.
    let moved = call(
        &both,
        "write_page",
        json!({ "title": "Rust", "body": "Ferris.", "base_revision": 3, "spheres": ["work"] }),
    )
    .await;
    assert_eq!(structured(&moved)["spheres"], json!(["work"]));
    let gone = error_text(&call(&private, "read_page", json!({ "title": "Rust" })).await);
    assert!(gone.contains("no page titled"), "{gone}");
}
