use reqwest::Client;
use serde::Serialize;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

#[derive(Serialize)]
struct WebhookPayload<'a> {
    username: &'a str,
    content: &'a str,
}

pub struct WebhookDispatcher {
    http: Client,
}

impl WebhookDispatcher {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
        }
    }

    /// Verstuurt een chatbericht naar een Discord Webhook met automatische 429 backoff retry
    pub async fn send_message(&self, webhook_url: &str, username: &str, content: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let payload = WebhookPayload { username, content };
        let max_retries = 3;

        for attempt in 0..max_retries {
            let resp = self.http.post(webhook_url)
                .json(&payload)
                .send()
                .await?;

            if resp.status().is_success() {
                return Ok(());
            }

            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                // Lees optionele retry-after header uit
                let wait_ms = resp.headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|secs| (secs * 1000.0) as u64)
                    .unwrap_or_else(|| 1000 * 2u64.pow(attempt as u32));

                warn!("Discord Webhook 429 Rate Limit! Wachten voor {} ms...", wait_ms);
                sleep(Duration::from_millis(wait_ms)).await;
                continue;
            }

            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("Fout bij versturen van Discord webhook ({}): {}", status, body);
            return Err(format!("Webhook fout {}: {}", status, body).into());
        }

        Err("Discord webhook verzending mislukt na 3 pogingen".into())
    }
}
