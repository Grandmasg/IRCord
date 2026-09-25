use std::time::{Duration, Instant};
use tokio::time::sleep;

pub struct TokenBucketLimiter {
    min_interval: Duration,
    last_sent: Instant,
}

impl TokenBucketLimiter {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            min_interval: Duration::from_millis(delay_ms),
            last_sent: Instant::now() - Duration::from_secs(10),
        }
    }

    /// Wacht totdat het veilig is om een nieuw bericht naar IRC te sturen om flood-kicks te voorkomen
    pub async fn wait_for_slot(&mut self) {
        let elapsed = self.last_sent.elapsed();
        if elapsed < self.min_interval {
            let wait_time = self.min_interval - elapsed;
            sleep(wait_time).await;
        }
        self.last_sent = Instant::now();
    }
}

/// Knipt lange AI antwoorden en berichten op in veilige IRC regels van max bytes
pub fn chunk_irc_message(text: &str, max_bytes: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let words = text.split_whitespace();
    let mut current_chunk = String::new();

    for word in words {
        if current_chunk.len() + word.len() + 1 > max_bytes {
            if !current_chunk.is_empty() {
                chunks.push(current_chunk);
            }
            current_chunk = word.to_string();
        } else if current_chunk.is_empty() {
            current_chunk = word.to_string();
        } else {
            current_chunk.push(' ');
            current_chunk.push_str(word);
        }
    }

    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_irc_message() {
        let text = "Dit is een lange zin die netjes opgeknipt moet worden in stukken";
        let chunks = chunk_irc_message(text, 20);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.len() <= 20);
        }
    }
}
