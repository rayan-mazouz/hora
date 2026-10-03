//! Request authentication: the credential helpers and the axum extractors
//! every handler starts from, so the "who is asking" preamble lives once.

use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use axum::extract::{FromRequestParts, MatchedPath, Query, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use serde::Deserialize;

use hora_core::config::Config;

use crate::AppState;
use crate::error::AppError;
use crate::visibility::Audience;

/// The header carrying a push monitor's, a peer's or a pushed alert's item
/// token. Preferred over `?token=` because it stays out of access logs.
pub(crate) const PUSH_TOKEN_HEADER: &str = "x-push-token";

/// The `Authorization: Bearer <token>` credential, if any.
pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

/// The `X-Push-Token` credential, if any.
pub(crate) fn push_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(PUSH_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
}

/// Constant-time string comparison so a wrong token can't be brute-forced by
/// timing. The length may leak (it is not the secret).
pub(crate) fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether `provided` matches the optional `expected` secret. No expected
/// secret never matches.
fn matches(provided: Option<&str>, expected: Option<&impl AsRef<str>>) -> bool {
    match (provided, expected) {
        (Some(provided), Some(expected)) => ct_eq(provided, expected.as_ref()),
        _ => false,
    }
}

#[derive(Debug, Default, Deserialize)]
struct TokenQuery {
    #[serde(default)]
    token: Option<String>,
}

/// The `Link` answered to a write authenticated with `?token=`: where its
/// deprecation is documented (RFC 9745's `deprecation` relation).
const QUERY_TOKEN_DEPRECATION_LINK: &str =
    "<https://uplg.github.io/hora/reference/api/#authentication>; rel=\"deprecation\"";

/// Set by a write endpoint whose caller authenticated with `?token=` rather
/// than a header; read back by [`deprecate_query_token`]. A shared flag rather
/// than a response value so it holds whatever the handler answers afterwards
/// (a 400 on a bad body still tells the client to move its token).
#[derive(Clone, Debug, Default)]
pub(crate) struct QueryTokenAuth(Arc<AtomicBool>);

impl QueryTokenAuth {
    /// Record that this request authenticated through `?token=`.
    pub(crate) fn mark(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn marked(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// The request's flag - a detached one (that nothing reads) on routes outside
/// [`deprecate_query_token`], such as the read-only views, where `?token=`
/// stays a first-class credential.
impl<S: Send + Sync> FromRequestParts<S> for QueryTokenAuth {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or_default()))
    }
}

/// Write-endpoint middleware: `?token=` still authenticates (for now), but a
/// request that relied on it is answered with `Deprecation: true` and a
/// `Link` to the documentation, and the first such request per endpoint is
/// logged - without the token, which is the very thing that should not be in
/// logs. Headers keep secrets out of access logs, proxies and browser history.
pub(crate) async fn deprecate_query_token(mut request: Request, next: Next) -> Response {
    let flag = QueryTokenAuth::default();
    request.extensions_mut().insert(flag.clone());
    let endpoint = format!(
        "{} {}",
        request.method(),
        request
            .extensions()
            .get::<MatchedPath>()
            .map_or_else(|| request.uri().path(), MatchedPath::as_str)
    );
    let mut response = next.run(request).await;
    if flag.marked() {
        let headers = response.headers_mut();
        headers.insert(
            HeaderName::from_static("deprecation"),
            HeaderValue::from_static("true"),
        );
        headers.append(
            header::LINK,
            HeaderValue::from_static(QUERY_TOKEN_DEPRECATION_LINK),
        );
        if WARNED_ENDPOINTS.first(&endpoint) {
            let header = if endpoint.contains("/api/push/") || endpoint.contains("/alert") {
                "Authorization: Bearer (or X-Push-Token)"
            } else {
                "Authorization: Bearer"
            };
            tracing::warn!(
                %endpoint,
                "a client authenticated with ?token=, which is deprecated on write endpoints: \
                 send the token in the {header} header instead"
            );
        }
    }
    response
}

/// The endpoints already warned about, so each logs once per process.
static WARNED_ENDPOINTS: LazyLock<WarnOnce> = LazyLock::new(WarnOnce::default);

/// A set of keys, each reported "first" exactly once.
#[derive(Default)]
struct WarnOnce(Mutex<HashSet<String>>);

impl WarnOnce {
    /// Whether this is the first time `key` is seen.
    fn first(&self, key: &str) -> bool {
        let mut seen = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        !seen.contains(key) && seen.insert(key.to_owned())
    }
}

/// The caller of a read-only view: the config snapshot the request is served
/// from and the audience its credential (Bearer, else `?token=`) resolves to.
/// With no `server.auth_token` configured nothing is private (config
/// validation enforces that), so every caller is [`Audience::Public`].
pub(crate) struct Viewer {
    pub(crate) config: Arc<Config>,
    pub(crate) audience: Audience,
    /// The presented credential (Bearer first, then `?token=`), kept so a
    /// group page can check it against its own group token.
    token: Option<String>,
    /// `?token=...` to append to in-page links (post-mortems, heatmap
    /// images), only when the operator authenticated by query: an `<img>` or
    /// a plain link cannot carry an Authorization header.
    pub(crate) token_query: String,
    /// Operator only through `?token=` (no matching Bearer header), and the
    /// request's flag to report it with on a write endpoint.
    operator_by_query: bool,
    query_auth: QueryTokenAuth,
}

impl Viewer {
    fn from_parts(parts: &Parts, config: Arc<Config>) -> Self {
        let query_token = Query::<TokenQuery>::try_from_uri(&parts.uri)
            .map(|Query(query)| query.token)
            .unwrap_or_default();
        let bearer = bearer(&parts.headers).map(str::to_owned);
        let expected = config.server.auth_token.as_ref();
        let by_bearer = matches(bearer.as_deref(), expected);
        let by_query = matches(query_token.as_deref(), expected);
        let audience = if by_bearer || by_query {
            Audience::Operator
        } else {
            Audience::Public
        };
        let token_query = query_token
            .as_deref()
            .filter(|_| by_query)
            .map(|token| format!("?token={}", hora_core::fmt::percent_encode(token)))
            .unwrap_or_default();
        Self {
            config,
            audience,
            token: bearer.or(query_token),
            token_query,
            operator_by_query: by_query && !by_bearer,
            query_auth: parts
                .extensions
                .get::<QueryTokenAuth>()
                .cloned()
                .unwrap_or_default(),
        }
    }

    pub(crate) fn is_operator(&self) -> bool {
        self.audience == Audience::Operator
    }

    /// A write endpoint authorized this request as the operator: flag it for
    /// the `?token=` deprecation when that is how it authenticated.
    pub(crate) fn note_operator_write(&self) {
        if self.operator_by_query {
            self.query_auth.mark();
        }
    }

    /// The audience on `group`'s own pages: the operator stays the operator,
    /// the group's `server.group_tokens` entry yields [`Audience::Group`], and
    /// anyone else is public. A group token is never accepted anywhere else.
    pub(crate) fn audience_for_group(&self, group: &str) -> Audience {
        if self.is_operator() {
            return Audience::Operator;
        }
        let expected = self.config.server.group_tokens.get(group);
        if matches(self.token.as_deref(), expected) {
            Audience::Group(group.to_owned())
        } else {
            Audience::Public
        }
    }
}

impl FromRequestParts<AppState> for Viewer {
    type Rejection = std::convert::Infallible;

    // Synchronous: everything is in the request head and the config snapshot.
    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let config = state.config.borrow().clone();
        std::future::ready(Ok(Self::from_parts(parts, config)))
    }
}

/// An operator action (announce, silence, event): requires the configured
/// `server.auth_token`. Without one configured the endpoint is closed - unlike
/// the read-only views, where "no token" just means "everything is public".
pub(crate) struct Operator {
    pub(crate) config: Arc<Config>,
}

impl FromRequestParts<AppState> for Operator {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let viewer = Viewer::from_parts(parts, state.config.borrow().clone());
        std::future::ready(if viewer.is_operator() {
            viewer.note_operator_write();
            Ok(Self {
                config: viewer.config,
            })
        } else {
            Err(AppError::Unauthorized(
                "this endpoint requires server.auth_token and a matching token",
            ))
        })
    }
}

/// Authenticate a mesh peer: it must be configured here, it must have a
/// `listen_token` (the id alone never authorizes), and `X-Push-Token` must
/// match. Unknown peers answer exactly like a wrong token. A function rather
/// than an extractor because `/api/peer/probe` names the peer in its JSON body.
pub(crate) fn authorize_peer(
    config: &Config,
    headers: &HeaderMap,
    from: &str,
) -> Result<(), AppError> {
    let expected = config
        .peers
        .iter()
        .find(|peer| peer.id == from)
        .and_then(|peer| peer.listen_token.as_ref());
    if matches(push_token(headers), expected) {
        Ok(())
    } else {
        Err(AppError::Unauthorized("unknown peer or invalid token"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_endpoint_warns_once() {
        let warned = WarnOnce::default();
        assert!(warned.first("POST /api/push/{id}"));
        assert!(!warned.first("POST /api/push/{id}"));
        assert!(warned.first("POST /api/event"));
        assert!(!warned.first("POST /api/event"));
    }
}
