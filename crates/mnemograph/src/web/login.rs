//! The owner's browser login: OIDC authorization code flow with PKCE against the identity
//! provider. Only members of the owner group get a session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mnemograph_core::Session;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{WebState, error_page};
use crate::auth::Authenticator;

pub const SESSION_COOKIE: &str = "mnemograph_session";
const LOGIN_COOKIE: &str = "mnemograph_login";
const SESSION_LIFETIME: Duration = Duration::from_secs(7 * 24 * 3600);
/// How long a started login may take until the provider redirects back.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

/// How the browser login is set up.
pub struct LoginSettings {
    /// Verifies ID tokens and knows the provider's endpoints.
    pub authenticator: Arc<Authenticator>,
    /// The web UI's client at the provider.
    pub client_id: String,
    /// Only for a confidential client; a public client relies on PKCE alone.
    pub client_secret: Option<String>,
    /// `<public-url>/auth/callback`, registered at the provider.
    pub redirect_uri: String,
    /// Members of this group (in the ID token's `groups` claim) are the owner.
    pub owner_group: String,
    /// Mark cookies `Secure` (when the public URL is https).
    pub secure_cookies: bool,
}

/// Logins that were started and wait for the provider's redirect, by `state`.
#[derive(Default)]
pub struct PendingLogins(Mutex<HashMap<String, PendingLogin>>);

struct PendingLogin {
    nonce: String,
    code_verifier: String,
    /// Where to go after the login.
    next: String,
    started: Instant,
}

impl PendingLogins {
    fn insert(&self, state: String, login: PendingLogin) {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        pending.retain(|_, login| login.started.elapsed() < LOGIN_TIMEOUT);
        pending.insert(state, login);
    }

    fn take(&self, state: &str) -> Option<PendingLogin> {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        pending
            .remove(state)
            .filter(|login| login.started.elapsed() < LOGIN_TIMEOUT)
    }
}

/// Middleware: lets the owner through (with their [`Session`] in the request extensions) and
/// sends everyone else to the login. Without a login configured, everyone is the owner —
/// development on a loopback address only.
pub async fn require_owner(
    State(state): State<Arc<WebState>>,
    mut request: Request,
    next: Next,
) -> Response {
    if state.login.is_none() {
        request.extensions_mut().insert(Session {
            owner_id: "anonymous".into(),
            owner_name: "anonymous".into(),
        });
        return next.run(request).await;
    }
    let session = match cookie(request.headers(), SESSION_COOKIE) {
        Some(token) => match state.knowledge.session(&token) {
            Ok(session) => session,
            Err(err) => {
                tracing::error!(%err, "reading the session failed");
                return error_page(StatusCode::INTERNAL_SERVER_ERROR, "Internal error");
            }
        },
        None => None,
    };
    match session {
        Some(session) => {
            request.extensions_mut().insert(session);
            next.run(request).await
        }
        None => {
            let next = request
                .uri()
                .path_and_query()
                .map_or("/".to_owned(), |pq| pq.as_str().to_owned());
            Redirect::to(&format!(
                "/login?next={}",
                percent_encoding::utf8_percent_encode(&next, percent_encoding::NON_ALPHANUMERIC)
            ))
            .into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct LoginQuery {
    next: Option<String>,
}

/// Starts a login: sends the browser to the provider.
pub async fn login(
    State(state): State<Arc<WebState>>,
    Query(query): Query<LoginQuery>,
) -> Response {
    let Some(login) = &state.login else {
        return Redirect::to("/").into_response();
    };
    let Some(endpoints) = login.authenticator.endpoints() else {
        return error_page(StatusCode::INTERNAL_SERVER_ERROR, "Login is not configured");
    };
    let login_state = random_token();
    let nonce = random_token();
    let code_verifier = random_token();
    let code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
    state.pending_logins.insert(
        login_state.clone(),
        PendingLogin {
            nonce: nonce.clone(),
            code_verifier,
            next: safe_next(query.next.as_deref()),
            started: Instant::now(),
        },
    );

    let mut url = match reqwest::Url::parse(&endpoints.authorization) {
        Ok(url) => url,
        Err(_) => return error_page(StatusCode::INTERNAL_SERVER_ERROR, "Invalid provider URL"),
    };
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &login.client_id)
        .append_pair("redirect_uri", &login.redirect_uri)
        .append_pair("scope", "openid email groups")
        .append_pair("state", &login_state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");

    // The cookie ties the provider's redirect to this browser (against login CSRF).
    let cookie = cookie_header(
        LOGIN_COOKIE,
        &login_state,
        LOGIN_TIMEOUT,
        login.secure_cookies,
    );
    ([(header::SET_COOKIE, cookie)], Redirect::to(url.as_str())).into_response()
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

/// Finishes a login: the provider redirected back with a code.
pub async fn callback(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let Some(login) = &state.login else {
        return Redirect::to("/").into_response();
    };
    if let Some(error) = query.error {
        let description = query.error_description.unwrap_or_default();
        return error_page(
            StatusCode::BAD_REQUEST,
            &format!("The login failed: {error} {description}"),
        );
    }
    let (Some(code), Some(login_state)) = (query.code, query.state) else {
        return error_page(StatusCode::BAD_REQUEST, "The login response is incomplete.");
    };
    if cookie(&headers, LOGIN_COOKIE).as_deref() != Some(login_state.as_str()) {
        return error_page(
            StatusCode::BAD_REQUEST,
            "This login was not started in this browser. Please log in again.",
        );
    }
    let Some(pending) = state.pending_logins.take(&login_state) else {
        return error_page(
            StatusCode::BAD_REQUEST,
            "This login has expired or was already used. Please log in again.",
        );
    };

    match finish_login(login, &code, &pending).await {
        Ok(session) => {
            if !session.1 {
                tracing::warn!(user = %session.0.owner_name, "login refused: not in the owner group");
                return error_page(
                    StatusCode::FORBIDDEN,
                    &format!(
                        "{} is not the owner of this mnemograph (not in the group {:?}).",
                        session.0.owner_name, login.owner_group
                    ),
                );
            }
            let token = match state.knowledge.create_session(&session.0, SESSION_LIFETIME) {
                Ok(token) => token,
                Err(err) => {
                    tracing::error!(%err, "creating the session failed");
                    return error_page(StatusCode::INTERNAL_SERVER_ERROR, "Internal error");
                }
            };
            tracing::info!(owner = %session.0.owner_name, "owner logged in");
            let mut response = Redirect::to(&pending.next).into_response();
            let headers = response.headers_mut();
            headers.append(
                header::SET_COOKIE,
                cookie_header(
                    SESSION_COOKIE,
                    &token,
                    SESSION_LIFETIME,
                    login.secure_cookies,
                ),
            );
            headers.append(
                header::SET_COOKIE,
                cookie_header(LOGIN_COOKIE, "", Duration::ZERO, login.secure_cookies),
            );
            response
        }
        Err(err) => {
            tracing::warn!(%err, "login failed");
            error_page(StatusCode::BAD_GATEWAY, &format!("The login failed: {err}"))
        }
    }
}

/// Exchanges the code for tokens and checks the ID token. Returns the session and whether the
/// user is in the owner group.
async fn finish_login(
    login: &LoginSettings,
    code: &str,
    pending: &PendingLogin,
) -> anyhow::Result<(Session, bool)> {
    let endpoints = login
        .authenticator
        .endpoints()
        .ok_or_else(|| anyhow::anyhow!("login is not configured"))?;
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &login.redirect_uri),
        ("client_id", &login.client_id),
        ("code_verifier", &pending.code_verifier),
    ];
    if let Some(secret) = &login.client_secret {
        form.push(("client_secret", secret));
    }
    let response = reqwest::Client::new()
        .post(&endpoints.token)
        .form(&form)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("the provider rejected the code ({status}): {body}");
    }
    let tokens: TokenResponse = response.json().await?;
    let claims = login
        .authenticator
        .verify_id_token(&tokens.id_token, &login.client_id)
        .await?;
    anyhow::ensure!(
        claims.nonce.as_deref() == Some(pending.nonce.as_str()),
        "the ID token does not belong to this login (nonce mismatch)"
    );
    let is_owner = claims.groups.contains(&login.owner_group);
    let owner_name = claims.email.clone().unwrap_or_else(|| claims.sub.clone());
    Ok((
        Session {
            owner_id: claims.sub,
            owner_name,
        },
        is_owner,
    ))
}

/// Ends the owner's session.
pub async fn logout(State(state): State<Arc<WebState>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie(&headers, SESSION_COOKIE)
        && let Err(err) = state.knowledge.end_session(&token)
    {
        tracing::error!(%err, "ending the session failed");
    }
    let secure = state.login.as_ref().is_some_and(|l| l.secure_cookies);
    (
        [(
            header::SET_COOKIE,
            cookie_header(SESSION_COOKIE, "", Duration::ZERO, secure),
        )],
        Redirect::to("/logged-out"),
    )
        .into_response()
}

/// Only local paths are allowed as targets after the login (no open redirect).
pub(super) fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(next) if next.starts_with('/') && !next.starts_with("//") && !next.contains('\\') => {
            next.to_owned()
        }
        _ => "/".to_owned(),
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(super) fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

pub(super) fn cookie_header(
    name: &str,
    value: &str,
    max_age: Duration,
    secure: bool,
) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        max_age.as_secs()
    ))
    .expect("cookie names and values are URL-safe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_paths_are_allowed_after_login() {
        assert_eq!(safe_next(Some("/pages/Rust")), "/pages/Rust");
        assert_eq!(safe_next(Some("https://evil.example")), "/");
        assert_eq!(safe_next(Some("//evil.example")), "/");
        assert_eq!(safe_next(Some("/\\evil.example")), "/");
        assert_eq!(safe_next(None), "/");
    }

    #[test]
    fn cookies_are_read_from_the_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "a=1; mnemograph_session=xyz; b=2".parse().unwrap(),
        );
        assert_eq!(cookie(&headers, SESSION_COOKIE).as_deref(), Some("xyz"));
        assert_eq!(cookie(&headers, "missing"), None);
    }
}
