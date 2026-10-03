//! ntfy notifier: POSTs a plain-text message to an ntfy topic.

use async_trait::async_trait;
use reqwest::Client;

use crate::message::{Kind, Message, Urgency};
use crate::util::{Unit, send_retrying};
use crate::{AlertSeverity, Event, Notifier};

/// ntfy turns a body over 4096 *bytes* into an attachment (or refuses it when
/// attachments are off), so the text is measured in bytes.
const HEAD_MAX: usize = 256;
const BODY_MAX: usize = 3800;

pub struct NtfyNotifier {
    client: Client,
    url: String,
    token: Option<String>,
}

impl NtfyNotifier {
    #[must_use]
    pub fn new(client: Client, url: String, token: Option<String>) -> Self {
        Self { client, url, token }
    }

    /// The body, the `Tags` header (an emoji shortcode) and the priority.
    fn message(event: Event<'_>) -> (String, &'static str, u8) {
        let message = Message::render(event).fit(HEAD_MAX, BODY_MAX, Unit::Bytes);
        (
            message.plain(),
            tag(message.kind),
            priority(message.urgency()),
        )
    }
}

/// ntfy shows the tag as an emoji in front of the title.
fn tag(kind: Kind) -> &'static str {
    match kind {
        Kind::Down | Kind::Alert(AlertSeverity::Error | AlertSeverity::Critical) => {
            "rotating_light"
        }
        Kind::Degraded | Kind::PeerLinkDegraded | Kind::Alert(AlertSeverity::Warning) => "warning",
        Kind::Recovered => "white_check_mark",
        Kind::CertExpiring | Kind::CertChanged | Kind::CertUnreadable => "lock",
        Kind::DomainExpiring => "globe_with_meridians",
        Kind::Release => "package",
        Kind::Digest => "bar_chart",
        Kind::BudgetBurn => "fire",
        Kind::Alert(AlertSeverity::Info) => "information_source",
    }
}

/// ntfy priority runs 1 (min) to 5 (max).
fn priority(urgency: Urgency) -> u8 {
    match urgency {
        Urgency::Quiet => 2,
        Urgency::Notice | Urgency::Warning => 3,
        Urgency::High => 4,
        Urgency::Critical => 5,
    }
}

#[async_trait]
impl Notifier for NtfyNotifier {
    fn name(&self) -> &'static str {
        "ntfy"
    }

    async fn notify(&self, event: Event<'_>) -> anyhow::Result<()> {
        let (message, tags, priority) = Self::message(event);
        let build = || {
            let mut req = self.client.post(&self.url).body(message.clone());
            req = req.header("Title", "Hora Alert");
            req = req.header("Tags", tags);
            req = req.header("Priority", priority.to_string());
            if let Some(token) = &self.token {
                req = req.bearer_auth(token);
            }
            req
        };
        // The topic URL itself is the capability; the bearer token must not
        // surface either if the server echoes it in a rejection body.
        let mut secrets = vec![self.url.as_str()];
        if let Some(token) = &self.token {
            secrets.push(token.as_str());
        }
        send_retrying(build, self.name(), &secrets).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_events_to_text_tags_and_priority() {
        let (text, tags, priority) = NtfyNotifier::message(Event::Down {
            monitor: "API",
            error: Some("boom"),
            cause: Some("DB"),
            impacted: &[],
            vantage: None,
            event: None,
        });
        assert_eq!(text, "API is DOWN\nboom\ncaused by DB");
        assert_eq!((tags, priority), ("rotating_light", 4));

        let (_, tags, priority) = NtfyNotifier::message(Event::Recovered { monitor: "API" });
        assert_eq!((tags, priority), ("white_check_mark", 2));

        let (text, tags, priority) = NtfyNotifier::message(Event::Alert {
            monitor: "API",
            severity: AlertSeverity::Critical,
            title: "disk full",
            message: "",
        });
        assert_eq!(text, "[CRITICAL] API: disk full");
        assert_eq!((tags, priority), ("rotating_light", 5));
    }

    #[test]
    fn long_bodies_stay_under_the_byte_limit() {
        let summary = "€".repeat(4000); // 12 000 bytes
        let (text, _, _) = NtfyNotifier::message(Event::Digest {
            period: "week",
            summary: &summary,
        });
        assert!(text.len() <= 4096, "{} bytes", text.len());
    }
}
