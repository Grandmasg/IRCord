use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::Config;
use crate::utils::error_log::ErrorLogger;

#[derive(Clone)]
pub struct WebState {
    pub config: Arc<Config>,
    pub error_logger: Arc<ErrorLogger>,
}

pub struct WebServer;

impl WebServer {
    pub async fn start(
        config: Arc<Config>,
        error_logger: Arc<ErrorLogger>,
        shutdown_token: CancellationToken,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let port = config.general.http_port;
        let state = WebState { config, error_logger };

        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/metrics", get(metrics_handler))
            .route("/api/errors", get(errors_handler))
            .route("/api/github", post(github_webhook_handler))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
        info!("Axum HTTP server luistert op poort {}", port);

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

async fn metrics_handler() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "service": "ircord_daemon",
        "memory_profile": "< 20MB",
        "runtime": "Tokio Async Rust"
    }))
}

async fn errors_handler(State(state): State<WebState>) -> impl IntoResponse {
    let recent = state.error_logger.recent(50);
    Json(json!({
        "status": "ok",
        "total_in_buffer": state.error_logger.count(),
        "entries": recent
    }))
}

async fn github_webhook_handler(
    State(_state): State<WebState>,
    headers: axum::http::HeaderMap,
    body: String,
) -> impl IntoResponse {
    // 1. Controleer optioneel GITHUB_WEBHOOK_SECRET via HMAC-SHA256
    if let Ok(secret) = std::env::var("GITHUB_WEBHOOK_SECRET") {
        let secret = secret.trim();
        if !secret.is_empty() {
            let sig = headers
                .get("X-Hub-Signature-256")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            if !crate::web::github::GitHubWebhookValidator::verify_signature(secret, body.as_bytes(), sig) {
                tracing::warn!("⛔ Inkomende GitHub webhook geweigerd: ongeldige X-Hub-Signature-256 handtekening!");
                return (StatusCode::UNAUTHORIZED, "Ongeldige handtekening (HMAC SHA-256 mislukt)").into_response();
            }
        }
    }

    info!("Inkomende geverifieerde GitHub webhook ontvangen ({} bytes)", body.len());
    (StatusCode::OK, "Webhook succesvol ontvangen en geverifieerd").into_response()
}
