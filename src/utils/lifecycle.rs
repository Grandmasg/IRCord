use tokio_util::sync::CancellationToken;
use tracing::info;

pub struct ShutdownManager {
    token: CancellationToken,
}

impl ShutdownManager {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    pub fn child_token(&self) -> CancellationToken {
        self.token.child_token()
    }

    pub fn trigger_shutdown(&self) {
        info!("Shutdown geactiveerd: opzeggen van actieve achtergrondtaken...");
        self.token.cancel();
    }

    pub async fn wait_for_signal(&self) {
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("Kon Ctrl+C signal handler niet initialiseren");
        };

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Kon SIGTERM signal handler niet initialiseren")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {
                info!("Ctrl+C ontvangen!");
            },
            _ = terminate => {
                info!("SIGTERM ontvangen!");
            },
            _ = self.token.cancelled() => {
                info!("Interne annulering ontvangen!");
            }
        }

        self.trigger_shutdown();
    }
}
