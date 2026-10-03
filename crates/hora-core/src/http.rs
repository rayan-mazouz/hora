//! Shared reqwest client construction (user-agent, timeout backstops, proxy),
//! the redirect policies, and bounded body reads.

use std::time::Duration;

use reqwest::redirect::{Attempt, Policy};
use reqwest::{Client, RequestBuilder, Response, Url};
use serde::de::DeserializeOwned;

/// How many redirects the shared client follows before giving up (reqwest's
/// own default).
const MAX_REDIRECTS: usize = 10;

/// Slack on top of a request's own deadline for the outer guard in
/// [`fetch_json_capped`]: reqwest's timeout already covers the body, this only
/// catches a stall the per-request timer somehow missed.
const OUTER_DEADLINE_SLACK: Duration = Duration::from_secs(2);

/// Build an HTTP client with Hora's user-agent and timeout backstops, optionally
/// routed through a `proxy` (`http(s)://…` or `socks5://…`). Used for notifiers
/// and for the peer mesh (heartbeats, witness polls, confirmation probes, the
/// vantage poll).
///
/// Follows redirects only within the request's origin (plus an `http` to
/// `https` upgrade on the same host): these requests carry custom secret
/// headers (`X-Push-Token`, `X-Gotify-Key`, ...) that reqwest does not strip on
/// cross-origin hops, so following one would hand the secret to whatever host a
/// misconfigured or compromised endpoint points at. A cross-origin redirect is
/// returned as-is (a 3xx response), which every caller treats as a failure.
///
/// # Errors
///
/// Returns an error if the proxy URL is invalid or the client cannot be built.
pub fn client(proxy: Option<&str>) -> reqwest::Result<Client> {
    build(proxy, Policy::custom(same_origin_redirect), None)
}

/// Like [`client`], but never auto-follows redirects: probes follow them
/// manually (see `probe::http`) so per-monitor credential headers are dropped on
/// cross-origin hops. reqwest strips only its own well-known sensitive headers
/// across hosts, so without this an arbitrary `X-Api-Key` would be re-sent to
/// whatever host a malicious target redirects to.
///
/// # Errors
///
/// Returns an error if the proxy URL is invalid or the client cannot be built.
pub fn probe_client(proxy: Option<&str>) -> reqwest::Result<Client> {
    build(proxy, Policy::none(), None)
}

/// Like [`probe_client`], but restricted to one address family by binding the
/// local end to `local` (the family's unspecified address): the connector then
/// only attempts resolved addresses of that family. Used by dual-stack probes;
/// never proxied - config validation rejects the combination, since through a
/// proxy only the proxy hop's family would be tested.
///
/// # Errors
///
/// Returns an error if the client cannot be built.
pub fn probe_client_family(local: std::net::IpAddr) -> reqwest::Result<Client> {
    build(None, Policy::none(), Some(local))
}

fn build(
    proxy: Option<&str>,
    redirect: Policy,
    local: Option<std::net::IpAddr>,
) -> reqwest::Result<Client> {
    let mut builder = Client::builder()
        .user_agent(concat!("hora/", env!("CARGO_PKG_VERSION")))
        .redirect(redirect)
        .local_address(local)
        // Backstop for requests without a per-request timeout (notifiers); probes
        // override this with the monitor's own timeout.
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10));
    if let Some(proxy) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    builder.build()
}

/// The redirect policy of [`client`]: follow a hop only while it stays on the
/// origin of the *initial* request (so a chain cannot launder its way out one
/// hop at a time), up to [`MAX_REDIRECTS`].
fn same_origin_redirect(attempt: Attempt) -> reqwest::redirect::Action {
    let allowed = attempt
        .previous()
        .first()
        .is_none_or(|initial| redirect_allowed(initial, attempt.url()));
    if !allowed {
        return attempt.stop();
    }
    if attempt.previous().len() > MAX_REDIRECTS {
        return attempt.error("too many redirects");
    }
    attempt.follow()
}

/// Whether a redirect from `initial` to `next` keeps the secret headers on the
/// same server: same scheme, host and port - or the same host upgraded from
/// `http` to `https` (the port then changes with the scheme, which is the
/// point of the upgrade, not a different server).
fn redirect_allowed(initial: &Url, next: &Url) -> bool {
    if initial.host_str() != next.host_str() {
        return false;
    }
    if initial.scheme() == next.scheme() {
        return initial.port_or_known_default() == next.port_or_known_default();
    }
    initial.scheme() == "http" && next.scheme() == "https" && next.port().is_none()
}

/// A response body read under a byte cap.
#[derive(Debug)]
pub struct CappedBody {
    /// At most `cap` bytes of the body.
    pub bytes: Vec<u8>,
    /// Whether the body was longer than the cap (the rest was not read).
    pub truncated: bool,
}

/// Read a response body, keeping at most `cap` bytes, so a huge (or hostile,
/// endless) body can't exhaust memory. Stops reading as soon as the cap is
/// passed and reports it in [`CappedBody::truncated`]; whether a truncated body
/// is still usable is the caller's call (a failure snapshot is, a JSON answer
/// is not).
///
/// # Errors
///
/// Returns the transport error when the body fails mid-stream: a partial body
/// is not a clean end of stream and must not be judged as if it were complete.
pub async fn read_capped(response: &mut Response, cap: usize) -> reqwest::Result<CappedBody> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        // Copy at most the remaining budget so one huge chunk can't blow the bound.
        let take = (cap - bytes.len()).min(chunk.len());
        bytes.extend_from_slice(&chunk[..take]);
        if take < chunk.len() {
            return Ok(CappedBody {
                bytes,
                truncated: true,
            });
        }
    }
    Ok(CappedBody {
        bytes,
        truncated: false,
    })
}

/// Send `request` and decode its JSON answer, bounded in time (`deadline`,
/// covering the body too) and size (`cap` bytes). Every failure - transport, a
/// non-2xx status (including an unfollowed redirect), an oversized, truncated
/// or malformed body, a timeout - is `None`: the mesh callers are all
/// fail-open, and an answer that could not be fully read must not be trusted.
pub async fn fetch_json_capped<T: DeserializeOwned>(
    request: RequestBuilder,
    cap: usize,
    deadline: Duration,
) -> Option<T> {
    let request = request.timeout(deadline);
    let fetch = async {
        let mut response = request.send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body = read_capped(&mut response, cap).await.ok()?;
        if body.truncated {
            return None;
        }
        serde_json::from_slice::<T>(&body.bytes).ok()
    };
    tokio::time::timeout(deadline + OUTER_DEADLINE_SLACK, fetch)
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    fn url(raw: &str) -> Url {
        raw.parse().expect("valid url")
    }

    #[test]
    fn redirects_stay_on_the_initial_origin() {
        let initial = url("https://peer.example/api/push/a");
        assert!(redirect_allowed(
            &initial,
            &url("https://peer.example/api/push/a/")
        ));
        assert!(redirect_allowed(
            &initial,
            &url("https://peer.example:443/elsewhere")
        ));
        // Another host, another port or a downgrade: refused.
        assert!(!redirect_allowed(&initial, &url("https://evil.example/")));
        assert!(!redirect_allowed(
            &initial,
            &url("https://peer.example:8443/")
        ));
        assert!(!redirect_allowed(&initial, &url("http://peer.example/")));
        // A subdomain is a different origin too.
        assert!(!redirect_allowed(&initial, &url("https://x.peer.example/")));
        // The same host upgraded to https (default port) is the same server.
        let plain = url("http://peer.example/api");
        assert!(redirect_allowed(&plain, &url("https://peer.example/api")));
        assert!(!redirect_allowed(
            &plain,
            &url("https://peer.example:8443/api")
        ));
    }

    /// A one-shot HTTP server answering every connection with `response`;
    /// returns its base URL and a counter of the requests that carried
    /// `x-push-token`.
    async fn serve(response: String) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let tokens = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = std::sync::Arc::clone(&tokens);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
                if head.contains("x-push-token") {
                    seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (format!("http://{addr}"), tokens)
    }

    #[tokio::test]
    async fn cross_origin_redirect_is_not_followed_with_the_token() {
        // The "attacker" origin: counts token-bearing requests.
        let (evil, evil_tokens) =
            serve("HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}".to_owned())
                .await;
        // The peer redirects to it (127.0.0.1 vs localhost: another host).
        let evil_host = evil.replace("127.0.0.1", "localhost");
        let (peer, _) = serve(format!(
            "HTTP/1.1 302 Found\r\nlocation: {evil_host}/steal\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        ))
        .await;

        let client = client(None).expect("client");
        let response = client
            .post(format!("{peer}/api/push/a"))
            .header("x-push-token", "s3cret")
            .send()
            .await
            .expect("response");
        // The 302 comes back unfollowed, and the other origin never saw the token.
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        assert_eq!(evil_tokens.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn fetch_json_capped_rejects_oversized_and_accepts_small() {
        let (small, _) = serve(
            "HTTP/1.1 200 OK\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"up\":true}"
                .to_owned(),
        )
        .await;
        let client = client(None).expect("client");
        let value: Option<serde_json::Value> =
            fetch_json_capped(client.get(&small), 64, Duration::from_secs(5)).await;
        assert_eq!(value, Some(serde_json::json!({"up": true})));

        // Same body, cap below its size: refused rather than parsed partially.
        let value: Option<serde_json::Value> =
            fetch_json_capped(client.get(&small), 4, Duration::from_secs(5)).await;
        assert_eq!(value, None);
    }
}
