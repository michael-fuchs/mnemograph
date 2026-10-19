//! End-to-end tests of OAuth: agents need a valid access token, and their identity becomes the
//! author of what they write.

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
    model::CallToolRequestParams,
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let settings = AuthSettings {
            issuer: ISSUER.into(),
            resource: format!("{base}/mcp"),
            required_scopes: vec!["mnemograph".into()],
        };
        let authenticator = Authenticator::with_keys(
            settings,
            HashMap::from([(
                KEY_ID.to_owned(),
                (DecodingKey::from_jwk(&keys.jwk).unwrap(), Algorithm::EdDSA),
            )]),
        );

        let dir = tempfile::tempdir().unwrap();
        let knowledge = Arc::new(Knowledge::open(dir.path().join("test.db")).unwrap());
        let shutdown = CancellationToken::new();
        let app = mnemograph::app(
            knowledge.clone(),
            AppConfig {
                auth: Some(Arc::new(authenticator)),
                allowed_hosts: vec!["127.0.0.1".into()],
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
            "scope": "openid mnemograph",
            "exp": now() + 300,
        })
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
        server.knowledge.page("Rust").unwrap().unwrap().author,
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
