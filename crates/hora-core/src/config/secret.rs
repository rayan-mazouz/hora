//! Secrets in the configuration, and the redaction that keeps them out of logs.

use serde::Deserialize;

/// Render a secret for `Debug`: `<unset>` when empty, `<redacted>` otherwise.
pub(super) fn redacted(secret: &str) -> &'static str {
    if secret.is_empty() {
        "<unset>"
    } else {
        "<redacted>"
    }
}

/// A configuration string that is redacted from `Debug` output, so a secret in a
/// `Debug`-derived struct (e.g. a monitor's push token) never reaches the logs.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(redacted(&self.0))
    }
}

// `AsRef`, not `Deref`: reading the secret must be explicit (`.as_ref()`), so it
// can't be coerced into a `Display` context (e.g. `info!("{}", *secret)`) by
// accident.
impl AsRef<str> for Secret {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Secret {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Mask the secrets a URL-like string may carry for `Debug` - `user:pass@`
/// credentials and query-string values (`?api_key=...`) - keeping the host,
/// path and query *keys* so logs stay useful. Inputs that don't parse as a URL
/// (e.g. a TCP `host:port` target) or carry neither are returned unchanged.
pub(super) fn redact_url_secrets(raw: &str) -> std::borrow::Cow<'_, str> {
    let Ok(mut url) = reqwest::Url::parse(raw) else {
        return std::borrow::Cow::Borrowed(raw);
    };
    let mut redacted = false;
    if !url.username().is_empty() || url.password().is_some() {
        // These setters only fail for cannot-be-a-base URLs, which never
        // carry credentials, so the guard above already excludes them.
        let _ = url.set_username("***");
        if url.password().is_some() {
            let _ = url.set_password(Some("***"));
        }
        redacted = true;
    }
    if url.query().is_some_and(|query| !query.is_empty()) {
        let keys: Vec<String> = url
            .query_pairs()
            .map(|(key, _value)| format!("{key}=***"))
            .collect();
        url.set_query(Some(&keys.join("&")));
        redacted = true;
    }
    if redacted {
        std::borrow::Cow::Owned(url.to_string())
    } else {
        std::borrow::Cow::Borrowed(raw)
    }
}
