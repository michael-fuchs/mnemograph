//! End-to-end tests of the web UI's login gate against a fake OIDC provider.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use mnemograph::web::login::LoginSettings;
use mnemograph::{AppConfig, AuthSettings, Authenticator};
use mnemograph_core::{Author, Knowledge, PageChanges};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

const CLIENT_ID: &str = "mnemograph-web";
const KEY_ID: &str = "web-test-key";

/// What the fake provider puts into the ID token for an authorization code.
#[derive(Clone)]
struct Grant {
    code_challenge: String,
    nonce: String,
    groups: Vec<String>,
}

#[derive(Clone)]
struct Provider {
    issuer: String,
    signing: Arc<EncodingKey>,
    grants: Arc<Mutex<HashMap<String, Grant>>>,
}

/// Starts a fake OIDC provider and returns it with its JWKS.
async fn start_provider() -> (Provider, CancellationToken) {
    let mut seed = [0u8; 32];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut seed);
    let key = SigningKey::from_bytes(&seed);
    let jwks = json!({ "keys": [{
        "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "kid": KEY_ID,
        "x": URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes()),
    }]});

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let provider = Provider {
        issuer: format!("{base}/auth/v1/"),
        signing: Arc::new(EncodingKey::from_ed_der(
            key.to_pkcs8_der().unwrap().as_bytes(),
        )),
        grants: Arc::default(),
    };
    let discovery = json!({
        "issuer": provider.issuer,
        "jwks_uri": format!("{base}/jwks"),
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
    });
    let app = Router::new()
        .route(
            "/auth/v1/.well-known/openid-configuration",
            get(move || async move { Json(discovery) }),
        )
        .route("/jwks", get(move || async move { Json(jwks) }))
        .route("/token", post(token))
        .with_state(provider.clone());
    let shutdown = CancellationToken::new();
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
                .unwrap();
        }
    });
    (provider, shutdown)
}

/// The fake token endpoint: checks PKCE and returns a signed ID token.
async fn token(
    State(provider): State<Provider>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Json<Value>, (axum::http::StatusCode, &'static str)> {
    let bad = |why| Err((axum::http::StatusCode::BAD_REQUEST, why));
    let Some(grant) = provider.grants.lock().unwrap().remove(&form["code"]) else {
        return bad("unknown code");
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes()));
    if challenge != grant.code_challenge || form["client_id"] != CLIENT_ID {
        return bad("PKCE or client mismatch");
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(KEY_ID.into());
    header.typ = Some("JWT".into());
    let id_token = jsonwebtoken::encode(
        &header,
        &json!({
            "iss": provider.issuer, "aud": CLIENT_ID, "sub": "user-1",
            "email": "owner@example.org", "exp": now + 300, "iat": now,
            "nonce": grant.nonce, "groups": grant.groups,
        }),
        &provider.signing,
    )
    .unwrap();
    Ok(Json(
        json!({ "access_token": "unused", "token_type": "Bearer", "id_token": id_token }),
    ))
}

struct Ui {
    base: String,
    http: reqwest::Client,
    provider: Provider,
    _shutdown: (CancellationToken, CancellationToken),
    _dir: tempfile::TempDir,
}

impl Ui {
    async fn start() -> Self {
        let (provider, provider_shutdown) = start_provider().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let authenticator = Arc::new(
            Authenticator::discover(AuthSettings {
                issuer: provider.issuer.clone(),
                resource: format!("{base}/mcp"),
                required_scopes: vec!["mnemograph".into()],
                spheres: mnemograph_core::Spheres::new(["work"]).unwrap(),
            })
            .await
            .unwrap(),
        );
        let dir = tempfile::tempdir().unwrap();
        let knowledge = Knowledge::open(dir.path().join("web.db")).unwrap();
        knowledge
            .write_page(
                "Rust",
                "A language. See [[Ferris]].\n\n<script>alert(1)</script>",
                None,
                PageChanges::default(),
                &mnemograph_core::Writer {
                    author: Author::Agent("claude-code".into()),
                    spheres: mnemograph_core::Spheres::new(["work"]).unwrap(),
                },
            )
            .unwrap();
        let shutdown = CancellationToken::new();
        let app = mnemograph::app(
            Arc::new(knowledge),
            AppConfig {
                auth: Some(authenticator.clone()),
                spheres: mnemograph_core::Spheres::new(["work"]).unwrap(),
                allowed_hosts: vec!["127.0.0.1".into()],
                limits: Default::default(),
                web_login: Some(LoginSettings {
                    authenticator,
                    client_id: CLIENT_ID.into(),
                    client_secret: None,
                    redirect_uri: format!("{base}/auth/callback"),
                    owner_group: "mnemograph_owner".into(),
                    secure_cookies: false,
                }),
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
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            provider,
            _shutdown: (shutdown, provider_shutdown),
            _dir: dir,
        }
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> reqwest::Response {
        let mut request = self.http.get(format!("{}{path}", self.base));
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        request.send().await.unwrap()
    }

    /// Starts a login and returns the login cookie and the authorization request's parameters.
    async fn start_login(&self, next: &str) -> (String, HashMap<String, String>) {
        let response = self.get(&format!("/login?next={next}"), None).await;
        assert_eq!(response.status(), 303);
        let cookie = set_cookie(&response, "mnemograph_login").expect("login cookie");
        let location = reqwest::Url::parse(location(&response)).unwrap();
        assert!(
            location
                .as_str()
                .starts_with(&self.provider.issuer.replace("/auth/v1/", "/authorize"))
        );
        (cookie, location.query_pairs().into_owned().collect())
    }

    /// Lets the fake provider accept the login for `params` with the given groups and nonce.
    fn grant(
        &self,
        code: &str,
        params: &HashMap<String, String>,
        groups: &[&str],
        nonce: Option<&str>,
    ) {
        self.provider.grants.lock().unwrap().insert(
            code.into(),
            Grant {
                code_challenge: params["code_challenge"].clone(),
                nonce: nonce.unwrap_or(&params["nonce"]).into(),
                groups: groups.iter().map(|g| g.to_string()).collect(),
            },
        );
    }

    async fn callback(&self, code: &str, state: &str, cookie: Option<&str>) -> reqwest::Response {
        self.get(&format!("/auth/callback?code={code}&state={state}"), cookie)
            .await
    }
}

fn location(response: &reqwest::Response) -> &str {
    response.headers()["location"].to_str().unwrap()
}

/// The `name=value` pair of a Set-Cookie header, if the response sets that cookie.
fn set_cookie(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap().to_owned())
        .find(|pair| pair.starts_with(&format!("{name}=")) && pair.len() > name.len() + 1)
}

#[tokio::test]
async fn the_owner_logs_in_and_sees_pages_rendered_safely() {
    let ui = Ui::start().await;

    let response = ui.get("/pages/Rust", None).await;
    assert_eq!(response.status(), 303);
    assert_eq!(location(&response), "/login?next=%2Fpages%2FRust");

    let (login_cookie, params) = ui.start_login("/pages/Rust").await;
    assert_eq!(params["client_id"], CLIENT_ID);
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["redirect_uri"], format!("{}/auth/callback", ui.base));
    ui.grant("code-1", &params, &["mnemograph_owner"], None);

    let response = ui
        .callback("code-1", &params["state"], Some(&login_cookie))
        .await;
    assert_eq!(response.status(), 303);
    assert_eq!(location(&response), "/pages/Rust");
    let session = set_cookie(&response, "mnemograph_session").expect("session cookie");

    let page = ui.get("/pages/Rust", Some(&session)).await;
    assert_eq!(page.status(), 200);
    let html = page.text().await.unwrap();
    assert!(html.contains("<h1>Rust</h1>"), "{html}");
    assert!(html.contains("claude-code (agent)"), "{html}");
    assert!(
        html.contains("title=\"Not written yet\">Ferris</a>"),
        "{html}"
    );
    assert!(html.contains("owner@example.org"), "{html}");
    assert!(
        !html.contains("<script>alert"),
        "raw HTML must not be rendered"
    );

    let start = ui.get("/", Some(&session)).await;
    assert!(start.text().await.unwrap().contains("href=\"/pages/Rust\""));
    let stub = ui.get("/pages/ferris", Some(&session)).await;
    assert!(stub.text().await.unwrap().contains("stub"));

    let logout = ui
        .http
        .post(format!("{}/logout", ui.base))
        .header("cookie", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(location(&logout), "/logged-out");
    assert_eq!(
        ui.get("/", Some(&session)).await.status(),
        303,
        "session ended"
    );
}

#[tokio::test]
async fn someone_outside_the_owner_group_is_refused() {
    let ui = Ui::start().await;
    let (cookie, params) = ui.start_login("/").await;
    ui.grant("code-2", &params, &["other_group"], None);
    let response = ui.callback("code-2", &params["state"], Some(&cookie)).await;
    assert_eq!(response.status(), 403);
    assert!(set_cookie(&response, "mnemograph_session").is_none());
}

#[tokio::test]
async fn a_callback_from_another_browser_or_a_replay_is_refused() {
    let ui = Ui::start().await;
    let (cookie, params) = ui.start_login("/").await;
    ui.grant("code-3", &params, &["mnemograph_owner"], None);

    let without_cookie = ui.callback("code-3", &params["state"], None).await;
    assert_eq!(
        without_cookie.status(),
        400,
        "login CSRF: not started in this browser"
    );

    let ok = ui.callback("code-3", &params["state"], Some(&cookie)).await;
    assert_eq!(ok.status(), 303);
    let replay = ui.callback("code-3", &params["state"], Some(&cookie)).await;
    assert_eq!(replay.status(), 400, "a state can only be used once");
}

#[tokio::test]
async fn an_id_token_for_another_login_is_refused() {
    let ui = Ui::start().await;
    let (cookie, params) = ui.start_login("/").await;
    ui.grant(
        "code-4",
        &params,
        &["mnemograph_owner"],
        Some("someone-elses-nonce"),
    );
    let response = ui.callback("code-4", &params["state"], Some(&cookie)).await;
    assert_eq!(response.status(), 502);
    assert!(response.text().await.unwrap().contains("nonce mismatch"));
}

#[tokio::test]
async fn open_redirects_after_login_are_not_possible() {
    let ui = Ui::start().await;
    let (cookie, params) = ui.start_login("https://evil.example").await;
    ui.grant("code-5", &params, &["mnemograph_owner"], None);
    let response = ui.callback("code-5", &params["state"], Some(&cookie)).await;
    assert_eq!(location(&response), "/");
}

#[tokio::test]
async fn an_agent_token_does_not_open_the_web_ui() {
    let ui = Ui::start().await;
    let response = ui
        .http
        .get(format!("{}/", ui.base))
        .bearer_auth("some-agent-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 303);
}
