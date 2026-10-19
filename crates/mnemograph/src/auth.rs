//! OAuth for agents: mnemograph is the resource server (ADR 0002).
//!
//! Agents get access tokens from the identity provider (Rauthy) with the authorization code flow
//! and send them as bearer tokens. This module validates them locally against the provider's
//! JWKS, serves the RFC 9728 Protected Resource Metadata that tells clients where to log in, and
//! answers requests without a valid token with a `401` challenge pointing there.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use mnemograph_core::Author;
use serde::Deserialize;
use tokio::sync::RwLock;

/// Path of the Protected Resource Metadata for the `/mcp` resource (RFC 9728 §3.1: the
/// resource's path is appended to the well-known prefix).
pub const METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
/// The token type of OAuth access tokens in JWT form (RFC 9068).
const ACCESS_TOKEN_TYPE: &str = "at+jwt";
/// Unknown key IDs trigger a JWKS refresh, but not more often than this.
const JWKS_MIN_REFRESH: Duration = Duration::from_secs(60);
const CLOCK_LEEWAY_SECS: u64 = 30;

/// Who made a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub author: Author,
}

/// How mnemograph checks tokens.
#[derive(Debug, Clone)]
pub struct AuthSettings {
    /// The issuer of access tokens, exactly as in their `iss` claim.
    pub issuer: String,
    /// mnemograph's MCP endpoint as clients reach it, e.g. `http://localhost:8080/mcp`; tokens
    /// must carry it as audience.
    pub resource: String,
    /// Scopes a token must have to use mnemograph.
    pub required_scopes: Vec<String>,
}

impl AuthSettings {
    /// Where clients find the Protected Resource Metadata: the well-known prefix goes between
    /// the host and the resource's path (RFC 9728 §3.1).
    pub fn metadata_url(&self) -> String {
        match reqwest::Url::parse(&self.resource) {
            Ok(url) => format!(
                "{}/.well-known/oauth-protected-resource{}",
                url.origin().ascii_serialization(),
                url.path().trim_end_matches('/')
            ),
            Err(_) => format!("{}{METADATA_PATH}", self.resource),
        }
    }
}

/// Validates access tokens.
pub struct Authenticator {
    settings: AuthSettings,
    keys: KeySource,
}

enum KeySource {
    /// Keys fetched from the provider's JWKS endpoint.
    Remote {
        jwks_url: String,
        http: reqwest::Client,
        cache: RwLock<KeyCache>,
    },
    /// Fixed keys, for tests.
    Fixed(HashMap<String, (DecodingKey, Algorithm)>),
}

#[derive(Default)]
struct KeyCache {
    keys: HashMap<String, (DecodingKey, Algorithm)>,
    fetched_at: Option<Instant>,
}

/// Why a request was not authenticated.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing bearer token")]
    MissingToken,
    #[error("invalid token: {0}")]
    InvalidToken(String),
    #[error("the token lacks the scopes {0:?}")]
    InsufficientScope(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: Option<String>,
    /// RFC 9068 names the client `client_id`; OIDC providers often use `azp`.
    client_id: Option<String>,
    azp: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(resource: &str) -> AuthSettings {
        AuthSettings {
            issuer: "http://localhost:8443/auth/v1".into(),
            resource: resource.into(),
            required_scopes: vec![],
        }
    }

    #[test]
    fn metadata_url_inserts_the_well_known_prefix_after_the_host() {
        assert_eq!(
            settings("http://localhost:8080/mcp").metadata_url(),
            "http://localhost:8080/.well-known/oauth-protected-resource/mcp"
        );
        assert_eq!(
            settings("https://kb.example.org/memory/mcp").metadata_url(),
            "https://kb.example.org/.well-known/oauth-protected-resource/memory/mcp"
        );
    }
}

#[derive(Debug, Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
}

impl Authenticator {
    /// Looks up the issuer's JWKS URL via OpenID Connect discovery.
    pub async fn discover(mut settings: AuthSettings) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let url = format!(
            "{}/.well-known/openid-configuration",
            settings.issuer.trim_end_matches('/')
        );
        let discovery: Discovery = http
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .with_context(|| format!("fetching {url}"))?
            .json()
            .await
            .with_context(|| format!("reading {url}"))?;
        // Providers differ on the trailing slash (Rauthy's issuer has one); tokens must carry
        // exactly the provider's own form.
        anyhow::ensure!(
            discovery.issuer.trim_end_matches('/') == settings.issuer.trim_end_matches('/'),
            "the provider at {url} calls itself {:?}, not {:?}",
            discovery.issuer,
            settings.issuer
        );
        settings.issuer = discovery.issuer;
        let authenticator = Self {
            settings,
            keys: KeySource::Remote {
                jwks_url: discovery.jwks_uri,
                http,
                cache: RwLock::default(),
            },
        };
        authenticator.refresh_keys(true).await?;
        Ok(authenticator)
    }

    /// An authenticator with fixed keys (key ID → key), for tests.
    pub fn with_keys(
        settings: AuthSettings,
        keys: HashMap<String, (DecodingKey, Algorithm)>,
    ) -> Self {
        Self {
            settings,
            keys: KeySource::Fixed(keys),
        }
    }

    pub fn settings(&self) -> &AuthSettings {
        &self.settings
    }

    /// Validates a bearer token and returns who sent it.
    pub async fn authenticate(&self, token: &str) -> Result<Identity, AuthError> {
        let invalid = |reason: &str| AuthError::InvalidToken(reason.to_owned());
        let header = jsonwebtoken::decode_header(token).map_err(|e| invalid(&e.to_string()))?;
        // Only access tokens: an ID token, also signed by the provider, must not work here.
        if !header
            .typ
            .as_deref()
            .is_some_and(|typ| typ.eq_ignore_ascii_case(ACCESS_TOKEN_TYPE))
        {
            return Err(invalid("not an access token (typ must be at+jwt)"));
        }
        let kid = header.kid.ok_or_else(|| invalid("no key ID"))?;
        let (key, algorithm) = self
            .key(&kid)
            .await
            .ok_or_else(|| invalid("unknown signing key"))?;
        if header.alg != algorithm {
            return Err(invalid("algorithm does not match the signing key"));
        }

        let mut validation = Validation::new(algorithm);
        validation.set_issuer(&[&self.settings.issuer]);
        validation.set_audience(&[&self.settings.resource]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        validation.leeway = CLOCK_LEEWAY_SECS;
        let claims = jsonwebtoken::decode::<Claims>(token, &key, &validation)
            .map_err(|e| invalid(&e.to_string()))?
            .claims;

        let granted: Vec<&str> = claims
            .scope
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        let missing: Vec<String> = self
            .settings
            .required_scopes
            .iter()
            .filter(|scope| !granted.contains(&scope.as_str()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(AuthError::InsufficientScope(missing));
        }

        let client = claims
            .client_id
            .or(claims.azp)
            .ok_or_else(|| invalid("no client ID (client_id or azp)"))?;
        tracing::debug!(%client, sub = ?claims.sub, "authenticated agent");
        Ok(Identity {
            author: Author::Agent(client),
        })
    }

    async fn key(&self, kid: &str) -> Option<(DecodingKey, Algorithm)> {
        match &self.keys {
            KeySource::Fixed(keys) => keys.get(kid).cloned(),
            KeySource::Remote { cache, .. } => {
                if let Some(key) = cache.read().await.keys.get(kid) {
                    return Some(key.clone());
                }
                // The provider may have rotated its keys.
                if let Err(err) = self.refresh_keys(false).await {
                    tracing::warn!(%err, "refreshing the JWKS failed");
                }
                cache.read().await.keys.get(kid).cloned()
            }
        }
    }

    async fn refresh_keys(&self, force: bool) -> anyhow::Result<()> {
        let KeySource::Remote {
            jwks_url,
            http,
            cache,
        } = &self.keys
        else {
            return Ok(());
        };
        let mut cache = cache.write().await;
        if !force
            && cache
                .fetched_at
                .is_some_and(|at| at.elapsed() < JWKS_MIN_REFRESH)
        {
            return Ok(());
        }
        cache.fetched_at = Some(Instant::now());
        let jwks: JwkSet = http
            .get(jwks_url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .with_context(|| format!("fetching {jwks_url}"))?
            .json()
            .await
            .with_context(|| format!("reading {jwks_url}"))?;
        cache.keys = decoding_keys(&jwks);
        tracing::info!(keys = cache.keys.len(), "loaded signing keys");
        Ok(())
    }
}

/// The usable keys of a JWKS by key ID; keys without ID or algorithm are skipped.
fn decoding_keys(jwks: &JwkSet) -> HashMap<String, (DecodingKey, Algorithm)> {
    jwks.keys
        .iter()
        .filter_map(|jwk| {
            let kid = jwk.common.key_id.clone()?;
            let algorithm = jwk.common.key_algorithm?.to_string().parse().ok()?;
            let key = DecodingKey::from_jwk(jwk).ok()?;
            Some((kid, (key, algorithm)))
        })
        .collect()
}

/// The RFC 9728 Protected Resource Metadata document.
pub fn metadata(settings: &AuthSettings) -> serde_json::Value {
    serde_json::json!({
        "resource": settings.resource,
        "authorization_servers": [settings.issuer],
        "scopes_supported": settings.required_scopes,
        "bearer_methods_supported": ["header"],
        "resource_name": "mnemograph",
    })
}

/// Middleware: lets requests with a valid bearer token through, with their [`Identity`] in the
/// request extensions; answers all others with a `401`/`403` challenge.
pub async fn require_token(
    State(auth): State<Arc<Authenticator>>,
    mut request: Request,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .map(str::trim);
    let result = match token {
        Some(token) => auth.authenticate(token).await,
        None => Err(AuthError::MissingToken),
    };
    match result {
        Ok(identity) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Err(err) => {
            tracing::debug!(%err, "rejected request");
            challenge(auth.settings(), &err)
        }
    }
}

/// Middleware for local development without authentication: every request is anonymous.
pub async fn anonymous(mut request: Request, next: Next) -> Response {
    request.extensions_mut().insert(Identity {
        author: Author::Anonymous,
    });
    next.run(request).await
}

/// A `401` (no or invalid token) or `403` (missing scope) response with a `WWW-Authenticate`
/// header that tells the client where to get a token (RFC 6750, RFC 9728 §5.1).
fn challenge(settings: &AuthSettings, err: &AuthError) -> Response {
    let mut value = format!("Bearer resource_metadata=\"{}\"", settings.metadata_url());
    if !settings.required_scopes.is_empty() {
        value += &format!(", scope=\"{}\"", settings.required_scopes.join(" "));
    }
    let status = match err {
        AuthError::MissingToken => StatusCode::UNAUTHORIZED,
        AuthError::InvalidToken(reason) => {
            value += &format!(
                ", error=\"invalid_token\", error_description=\"{}\"",
                reason.replace(['"', '\\'], "'")
            );
            StatusCode::UNAUTHORIZED
        }
        AuthError::InsufficientScope(_) => {
            value += ", error=\"insufficient_scope\"";
            StatusCode::FORBIDDEN
        }
    };
    let mut response = (status, err.to_string()).into_response();
    if let Ok(value) = HeaderValue::from_str(&value) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}
