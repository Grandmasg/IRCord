use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::Config;
use crate::utils::error_log::ErrorLogger;

#[derive(Clone)]
pub struct WebState {
    pub config: Arc<Config>,
    pub error_logger: Arc<ErrorLogger>,
    pub started: Instant,
    pub plugin_count: usize,
    /// Meldingen (bijv. GitHub events) die naar alle gekoppelde kanalen gestuurd worden.
    pub announce_tx: mpsc::Sender<String>,
}

pub struct WebServer;

impl WebServer {
    pub async fn start(
        config: Arc<Config>,
        error_logger: Arc<ErrorLogger>,
        plugin_count: usize,
        announce_tx: mpsc::Sender<String>,
        shutdown_token: CancellationToken,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let port = config.general.http_port;
        let bind = config.general.http_bind.clone();
        let state = WebState { config, error_logger, started: Instant::now(), plugin_count, announce_tx };

        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/metrics", get(metrics_handler))
            .route("/api/errors", get(errors_handler))
            .route("/api/github", post(github_webhook_handler))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(format!("{}:{}", bind, port)).await?;
        info!("Axum HTTP server luistert op {}:{}", bind, port);

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_token.cancelled().await;
            })
            .await?;

        info!("Axum HTTP server netjes afgesloten.");
        Ok(())
    }
}

async fn health_handler() -> impl IntoResponse {
    Json(json!({ "status": "ok", "service": "ircord" }))
}

/// Resident geheugen van dit proces in bytes (alleen Linux/containers; elders `None`).
fn resident_memory_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb = status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    Some(kb * 1024)
}

async fn metrics_handler(State(state): State<WebState>) -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "service": "ircord_daemon",
        "uptime_seconds": state.started.elapsed().as_secs(),
        "plugins": state.plugin_count,
        "errors_in_buffer": state.error_logger.count(),
        "memory_rss_bytes": resident_memory_bytes(),
        "runtime": "Tokio Async Rust"
    }))
}

/// Constante-tijd vergelijking om timing-lekken bij tokenvergelijking te voorkomen.
fn token_matches(expected: &str, provided: &str) -> bool {
    let (a, b) = (expected.as_bytes(), provided.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (*a.get(i).unwrap_or(&0) ^ *b.get(i).unwrap_or(&0)) as usize;
    }
    diff == 0
}

/// `/api/errors` bevat interne foutmeldingen en vereist `Authorization: Bearer $HTTP_API_TOKEN`.
fn api_authorized(headers: &axum::http::HeaderMap) -> Result<(), (StatusCode, &'static str)> {
    let expected = std::env::var("HTTP_API_TOKEN").unwrap_or_default();
    let expected = expected.trim();
    if expected.is_empty() {
        return Err((StatusCode::FORBIDDEN, "HTTP_API_TOKEN is niet ingesteld; dit endpoint is uitgeschakeld"));
    }
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if token_matches(expected, provided.trim()) {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, "Ongeldig of ontbrekend token"))
    }
}

async fn errors_handler(State(state): State<WebState>, headers: axum::http::HeaderMap) -> impl IntoResponse {
    if let Err((code, msg)) = api_authorized(&headers) {
        return (code, msg).into_response();
    }
    let recent = state.error_logger.recent(50);
    Json(json!({
        "status": "ok",
        "total_in_buffer": state.error_logger.count(),
        "entries": recent
    }))
    .into_response()
}

async fn github_webhook_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    body: String,
) -> impl IntoResponse {
    // De handtekening is verplicht: zonder geheim is het endpoint uitgeschakeld
    let secret = std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default();
    let secret = secret.trim();
    if secret.is_empty() || secret.starts_with("your_") {
        return (StatusCode::SERVICE_UNAVAILABLE, "GITHUB_WEBHOOK_SECRET is niet ingesteld; webhook uitgeschakeld").into_response();
    }

    let sig = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !crate::web::github::GitHubWebhookValidator::verify_signature(secret, body.as_bytes(), sig) {
        tracing::warn!("⛔ Inkomende GitHub webhook geweigerd: ongeldige X-Hub-Signature-256 handtekening!");
        return (StatusCode::UNAUTHORIZED, "Ongeldige handtekening (HMAC SHA-256 mislukt)").into_response();
    }

    let event = headers.get("X-GitHub-Event").and_then(|v| v.to_str().ok()).unwrap_or("");
    info!("Geverifieerde GitHub webhook '{}' ontvangen ({} bytes)", event, body.len());
    if event == "ping" {
        return (StatusCode::OK, "pong").into_response();
    }

    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "Ongeldige JSON").into_response();
    };
    let dutch = state.config.general.language == "nl";
    if let Some(line) = crate::web::github::format_event(event, &payload, dutch) {
        let _ = state.announce_tx.try_send(line);
    }
    (StatusCode::OK, "Webhook succesvol ontvangen en geverifieerd").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        assert!(!token_matches("abc123", "abc12"));
        assert!(!token_matches("abc123", ""));
    }
}
