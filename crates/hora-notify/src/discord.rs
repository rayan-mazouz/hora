//! Discord incoming-webhook notifier.

use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;

use crate::message::{Kind, Markup, Message};
use crate::util::{Unit, post_json};
use crate::{AlertSeverity, Event, Notifier};

// Embed accent colours, matching the status badges: red / green / orange.
const COLOR_DOWN: u32 = 0x00E0_5D44;
const COLOR_UP: u32 = 0x0044_CC11;
const COLOR_CERT: u32 = 0x00FE_7D37;
const COLOR_DEGRADED: u32 = 0x00DF_B317;
const COLOR_INFO: u32 = 0x0035_82F6;

/// Discord markdown for the embed description. Text is passed as is (an embed
/// never pings); a backtick inside code would close the fence early.
const MARKDOWN: Markup = Markup {
    text: str::to_owned,
    code: neutralise_backticks,
    bold: ("", ""),
    block: ("```", "```"),
    inline: ("`", "`"),
};

fn neutralise_backticks(code: &str) -> String {
    code.replace('`', "'")
}

/// Embed limits: title 256 characters, description 4096. The margins cover
/// the icon in the title and the fences in the description.
const TITLE_MAX: usize = 250;
const DESCRIPTION_MAX: usize = 4000;

/// Posts alerts to a Discord channel through an incoming webhook.
pub struct DiscordNotifier {
    client: Client,
    webhook_url: String,
}

impl DiscordNotifier {
    #[must_use]
    pub fn new(client: Client, webhook_url: String) -> Self {
        Self {
            client,
            webhook_url,
        }
    }

    fn embed(event: Event<'_>) -> Embed {
        let message = Message::render(event).fit(TITLE_MAX, DESCRIPTION_MAX, Unit::Chars);
        let description = message.body_with(&MARKDOWN);
        Embed {
            // Titles take no markdown: plain headline behind the icon.
            title: format!("{} {}", message.kind.icon(), message.headline()),
            description: (!description.is_empty()).then_some(description),
            color: color(message.kind),
        }
    }
}

fn color(kind: Kind) -> u32 {
    match kind {
        Kind::Down | Kind::Alert(AlertSeverity::Error | AlertSeverity::Critical) => COLOR_DOWN,
        Kind::Recovered | Kind::Digest => COLOR_UP,
        Kind::CertExpiring | Kind::DomainExpiring | Kind::Release | Kind::CertChanged => COLOR_CERT,
        Kind::Degraded
        | Kind::PeerLinkDegraded
        | Kind::BudgetBurn
        | Kind::Alert(AlertSeverity::Warning) => COLOR_DEGRADED,
        Kind::Alert(AlertSeverity::Info) => COLOR_INFO,
    }
}

/// The rendered embed, before borrowing into the JSON payload.
struct Embed {
    title: String,
    description: Option<String>,
    color: u32,
}

#[derive(Serialize)]
struct Payload<'a> {
    embeds: [EmbedJson<'a>; 1],
}

#[derive(Serialize)]
struct EmbedJson<'a> {
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    color: u32,
}

#[async_trait]
impl Notifier for DiscordNotifier {
    fn name(&self) -> &'static str {
        "discord"
    }

    async fn notify(&self, event: Event<'_>) -> anyhow::Result<()> {
        let embed = Self::embed(event);
        let payload = Payload {
            embeds: [EmbedJson {
                title: &embed.title,
                description: embed.description.as_deref(),
                color: embed.color,
            }],
        };
        post_json(
            &self.client,
            &self.webhook_url,
            &payload,
            self.name(),
            &[self.webhook_url.as_str()],
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_render_per_event() {
        let down = DiscordNotifier::embed(Event::Down {
            monitor: "API",
            error: Some("boom"),
            cause: Some("DB"),
            impacted: &[],
            vantage: None,
            event: None,
        });
        assert_eq!(down.title, "\u{1F534} API is DOWN");
        assert_eq!(
            down.description.as_deref(),
            Some("```boom```\ncaused by DB")
        );
        assert_eq!(down.color, COLOR_DOWN);

        let recovered = DiscordNotifier::embed(Event::Recovered { monitor: "API" });
        assert!(recovered.title.contains("recovered"));
        assert!(recovered.description.is_none());
        assert_eq!(recovered.color, COLOR_UP);

        let cert = DiscordNotifier::embed(Event::CertExpiring {
            monitor: "API",
            days_left: 3,
        });
        assert!(cert.title.contains("expires in 3 days"));
        assert_eq!(cert.color, COLOR_CERT);

        let degraded = DiscordNotifier::embed(Event::Degraded {
            monitor: "API",
            latency_ms: Some(1234),
            detail: None,
        });
        assert!(degraded.title.contains("slow") && degraded.title.contains("1234ms"));
        assert_eq!(degraded.color, COLOR_DEGRADED);

        let alert = DiscordNotifier::embed(Event::Alert {
            monitor: "API",
            severity: AlertSeverity::Info,
            title: "deploy",
            message: "v2",
        });
        assert_eq!(alert.title, "\u{1F514} [INFO] API: deploy");
        assert_eq!(alert.description.as_deref(), Some("v2"));
        assert_eq!(alert.color, COLOR_INFO);
    }

    #[test]
    fn down_body_neutralises_backticks() {
        let down = DiscordNotifier::embed(Event::Down {
            monitor: "API",
            error: Some("``` injection"),
            cause: None,
            impacted: &[],
            vantage: None,
            event: None,
        });
        let body = down.description.expect("down has a body");
        assert!(
            body.contains("''' injection"),
            "backticks not stripped: {body}"
        );
    }

    #[test]
    fn long_alerts_fit_the_embed_limits() {
        let name = "n".repeat(100);
        let title = "t".repeat(200);
        let summary = "s".repeat(10_000);
        let alert = DiscordNotifier::embed(Event::Alert {
            monitor: &name,
            severity: AlertSeverity::Critical,
            title: &title,
            message: &summary,
        });
        assert!(alert.title.chars().count() <= 256);
        let description = alert.description.expect("has a body");
        assert!(description.chars().count() <= 4096);

        let digest = DiscordNotifier::embed(Event::Digest {
            period: "week",
            summary: &summary,
        });
        assert!(digest.description.expect("has a body").chars().count() <= 4096);
    }
}
