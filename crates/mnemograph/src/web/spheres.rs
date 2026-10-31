//! The owner's sphere filter: which spheres the page list, search, graph and statistics show.
//!
//! The choice is kept in a cookie, so it stays while navigating and live updates (whose
//! requests carry it too) render with it. Without a choice, everything is shown.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{FromRequestParts, RawQuery, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Redirect, Response};
use mnemograph_core::{Spheres, Visibility, is_sphere_name};

use super::WebState;
use super::login::{cookie, cookie_header, safe_next};

const COOKIE: &str = "mnemograph_spheres";
/// Separates sphere names in the cookie; it never occurs in a name.
const SEPARATOR: char = '.';
/// How long the choice is kept.
const KEEP: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// The chosen spheres; `None` shows everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SphereFilter(Option<Spheres>);

impl SphereFilter {
    fn parse(names: impl IntoIterator<Item = String>) -> Self {
        Self(Spheres::new(names.into_iter().filter(|n| is_sphere_name(n))).ok())
    }

    pub fn visibility(&self) -> Visibility<'_> {
        match &self.0 {
            Some(spheres) => Visibility::Within(spheres),
            None => Visibility::All,
        }
    }

    pub fn contains(&self, sphere: &str) -> bool {
        self.0.as_ref().is_some_and(|s| s.contains(sphere))
    }

    pub fn is_active(&self) -> bool {
        self.0.is_some()
    }

    /// The chosen spheres for the header, e.g. "work, private", or "all".
    pub fn summary(&self) -> String {
        match &self.0 {
            Some(spheres) => spheres.names().join(", "),
            None => "all".to_owned(),
        }
    }

    fn from_headers(headers: &HeaderMap) -> Self {
        let value = cookie(headers, COOKIE).unwrap_or_default();
        Self::parse(value.split(SEPARATOR).map(str::to_owned))
    }
}

impl<S: Send + Sync> FromRequestParts<S> for SphereFilter {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_headers(&parts.headers))
    }
}

/// Sets the filter to the `sphere` parameters (none: everything) and returns to the page the
/// form was sent from.
pub async fn choose(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let chosen = SphereFilter::parse(
        query
            .as_deref()
            .unwrap_or("")
            .split('&')
            .filter_map(|pair| pair.strip_prefix("sphere="))
            .map(str::to_owned),
    );
    let secure = state.login.as_ref().is_some_and(|l| l.secure_cookies);
    let cookie = match &chosen.0 {
        Some(spheres) => cookie_header(
            COOKIE,
            &spheres.names().join(&SEPARATOR.to_string()),
            KEEP,
            secure,
        ),
        None => cookie_header(COOKIE, "", Duration::ZERO, secure),
    };
    (
        [(header::SET_COOKIE, cookie)],
        Redirect::to(&back(&headers)),
    )
        .into_response()
}

/// Where the filter form was sent from: the local path and query of the `Referer`, or `/`.
fn back(headers: &HeaderMap) -> String {
    let referer = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| reqwest::Url::parse(value).ok());
    let local = referer.map(|url| match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    });
    safe_next(local.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn the_filter_is_read_from_its_cookie_ignoring_invalid_names() {
        let mut headers = HeaderMap::new();
        assert_eq!(SphereFilter::from_headers(&headers), SphereFilter(None));
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; mnemograph_spheres=work.private.Bad%20one"),
        );
        let filter = SphereFilter::from_headers(&headers);
        assert!(filter.contains("work") && filter.contains("private"));
        assert_eq!(filter.summary(), "private, work");
    }

    #[test]
    fn the_form_returns_only_to_local_pages() {
        let back_from = |referer: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::REFERER, HeaderValue::from_static(referer));
            back(&headers)
        };
        assert_eq!(
            back_from("http://localhost:8080/search?q=rust"),
            "/search?q=rust"
        );
        assert_eq!(back_from("http://evil.example//x"), "/");
        assert_eq!(back_from("not a url"), "/");
        assert_eq!(back(&HeaderMap::new()), "/");
    }
}
