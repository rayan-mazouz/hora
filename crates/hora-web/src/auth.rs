//! Request authentication: the credential helpers and the axum extractors
//! every handler starts from, so the "who is asking" preamble lives once.

use std::future::Future;
use std::sync::Arc;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
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
        }
    }

    pub(crate) fn is_operator(&self) -> bool {
        self.audience == Audience::Operator
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
