//! Eenvoudige glijdende-venster limiet per gebruiker (voor dure commando's zoals !ai, !http, !dns).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Commando's die externe diensten of de lokale AI belasten.
pub const EXPENSIVE_COMMANDS: &[&str] = &[
    "ai", "tldr", "summary", "roast", "rant", "tirade", "whatis", "def", "catchup", "digest", "vibe", "sentiment",
    "http", "ssl", "dns", "tr", "translate", "vertaal", "yt", "youtube", "g", "google", "search", "img", "upload", "regen", "rain", "buien", "postcode", "ipinfo", "domein", "domain", "define", "short", "xkcd", "joke", "grap", "chatsearch", "chatzoek", "wiezei",
];

pub struct UserRateLimiter {
    limit: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl UserRateLimiter {
    /// `limit` aanroepen per `window`; `limit == 0` schakelt de limiet uit.
    pub fn new(limit: u32, window: Duration) -> Self {
        Self { limit: limit as usize, window, hits: Mutex::new(HashMap::new()) }
    }

    /// `Ok(())` als de aanroep mag; `Err(seconden)` met de wachttijd als de limiet is bereikt.
    pub fn check(&self, key: &str, now: Instant) -> Result<(), u64> {
        if self.limit == 0 {
            return Ok(());
        }
        let mut map = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 5000 {
            map.retain(|_, q| q.back().map(|t| now.duration_since(*t) < self.window).unwrap_or(false));
        }
        let q = map.entry(key.to_lowercase()).or_default();
        while q.front().map(|t| now.duration_since(*t) >= self.window).unwrap_or(false) {
            q.pop_front();
        }
        if q.len() >= self.limit {
            let wait = self.window.saturating_sub(now.duration_since(*q.front().unwrap()));
            return Err(wait.as_secs().max(1));
        }
        q.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_limit_and_recovers() {
        let rl = UserRateLimiter::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(rl.check("irc:Pjot", t0 + Duration::from_secs(i)).is_ok());
        }
        let wait = rl.check("irc:pjot", t0 + Duration::from_secs(10)).unwrap_err();
        assert!((49..=50).contains(&wait), "{wait}");
        // andere gebruiker heeft eigen teller
        assert!(rl.check("irc:henk", t0 + Duration::from_secs(10)).is_ok());
        // na het venster mag het weer
        assert!(rl.check("irc:pjot", t0 + Duration::from_secs(61)).is_ok());
    }

    #[test]
    fn zero_disables() {
        let rl = UserRateLimiter::new(0, Duration::from_secs(60));
        let t0 = Instant::now();
        for _ in 0..100 {
            assert!(rl.check("x", t0).is_ok());
        }
    }
}
