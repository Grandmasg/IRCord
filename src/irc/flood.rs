use std::collections::{HashMap, VecDeque};
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

/// Detecteert join-floods (raids/clone-aanvallen): `threshold` joins binnen `window` in één kanaal.
pub struct RaidGuard {
    threshold: usize,
    window: Duration,
    joins: HashMap<String, VecDeque<Instant>>,
}

impl RaidGuard {
    pub fn new(threshold: u32, window: Duration) -> Self {
        Self { threshold: threshold.max(2) as usize, window, joins: HashMap::new() }
    }

    /// Registreert een join; geeft `true` zodra de drempel voor dit kanaal wordt bereikt.
    /// De teller wordt daarna geleegd, zodat één raid maar één keer meldt.
    pub fn record_join(&mut self, channel: &str, now: Instant) -> bool {
        let q = self.joins.entry(channel.to_lowercase()).or_default();
        q.push_back(now);
        while q.front().map(|t| now.duration_since(*t) > self.window).unwrap_or(false) {
            q.pop_front();
        }
        if q.len() >= self.threshold {
            q.clear();
            true
        } else {
            false
        }
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

#[cfg(test)]
mod raid_tests {
    use super::*;

    #[test]
    fn triggers_on_burst_only() {
        let mut g = RaidGuard::new(3, Duration::from_secs(1));
        let t0 = Instant::now();
        assert!(!g.record_join("#a", t0));
        assert!(!g.record_join("#a", t0 + Duration::from_millis(200)));
        assert!(g.record_join("#a", t0 + Duration::from_millis(400)));
        // na de melding begint de teller opnieuw
        assert!(!g.record_join("#a", t0 + Duration::from_millis(500)));
    }

    #[test]
    fn slow_joins_and_other_channels_do_not_trigger() {
        let mut g = RaidGuard::new(3, Duration::from_secs(1));
        let t0 = Instant::now();
        for i in 0..6 {
            assert!(!g.record_join("#a", t0 + Duration::from_secs(2 * i)));
        }
        assert!(!g.record_join("#b", t0));
        assert!(!g.record_join("#c", t0));
        assert!(!g.record_join("#b", t0));
    }
}
