use sqlx::SqlitePool;
use std::time::Duration;
use tokio::sync::mpsc::{channel, Sender};
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub struct ChatLogEntry {
    pub channel: String,
    pub author: String,
    pub platform: String,
    pub message: String,
    pub timestamp: String,
}

pub struct RagSearcher {
    pool: SqlitePool,
    batch_tx: Sender<ChatLogEntry>,
}

impl RagSearcher {
    pub fn new(pool: SqlitePool) -> Self {
        let (batch_tx, mut batch_rx) = channel::<ChatLogEntry>(1024);
        let worker_pool = pool.clone();

        // Achtergrond worker: batched writes per 1 seconde of bij 50 berichten
        // Dit reduceert I/O-locks en SSD/SD wear met >95%
        tokio::spawn(async move {
            let mut buffer = Vec::with_capacity(50);
            let mut interval = tokio::time::interval(Duration::from_secs(1));

            loop {
                tokio::select! {
                    Some(entry) = batch_rx.recv() => {
                        buffer.push(entry);
                        if buffer.len() >= 50 {
                            Self::flush_batch(&worker_pool, &mut buffer).await;
                        }
                    }
                    _ = interval.tick() => {
                        if !buffer.is_empty() {
                            Self::flush_batch(&worker_pool, &mut buffer).await;
                        }
                    }
                }
            }
        });

        Self { pool, batch_tx }
    }

    async fn flush_batch(pool: &SqlitePool, buffer: &mut Vec<ChatLogEntry>) {
        if buffer.is_empty() {
            return;
        }

        let mut tx = match pool.begin().await {
            Ok(t) => t,
            Err(e) => {
                error!("Fout bij openen van SQLite transactie voor FTS5 batch: {}", e);
                return;
            }
        };

        for item in buffer.drain(..) {
            let res = sqlx::query!(
                r#"
                INSERT INTO chat_history (channel, author, platform, message, timestamp)
                VALUES (?, ?, ?, ?, ?)
                "#,
                item.channel,
                item.author,
                item.platform,
                item.message,
                item.timestamp
            )
            .execute(&mut *tx)
            .await;

            if let Err(e) = res {
                error!("Fout bij inserten van chatlog in batch: {}", e);
            }
        }

        if let Err(e) = tx.commit().await {
            error!("Fout bij committen van FTS5 batch transactie: {}", e);
        }
    }

    /// Zoekt recente chatberichten via SQLite FTS5 die matchen met een zoekterm
    pub async fn search_history(&self, query: &str, limit: i64) -> Result<Vec<String>, sqlx::Error> {
        let sanitized = query.replace('"', "").replace('*', "");
        let fts_query = format!("\"{}\"", sanitized);

        debug!("FTS5 chatgeschiedenis doorzoeken naar: {}", fts_query);

        let rows: Vec<(String, String, String)> = sqlx::query_as(
            r#"
            SELECT author, message, timestamp
            FROM chat_history
            WHERE chat_history MATCH ?
            ORDER BY rank
            LIMIT ?
            "#,
        )
        .bind(fts_query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let results = rows
            .into_iter()
            .map(|(author, message, timestamp)| format!("[{}] {}: {}", timestamp, author, message))
            .collect();

        Ok(results)
    }

    /// Stuurt een nieuw chatbericht direct door naar de asynchrone batching-queue
    pub async fn log_message(&self, channel: &str, author: &str, platform: &str, message: &str) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let entry = ChatLogEntry {
            channel: channel.to_string(),
            author: author.to_string(),
            platform: platform.to_string(),
            message: message.to_string(),
            timestamp: now,
        };

        let _ = self.batch_tx.send(entry).await;
        Ok(())
    }
}
