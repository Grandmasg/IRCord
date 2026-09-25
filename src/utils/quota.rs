use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::warn;

#[derive(Debug, Clone)]
pub struct QuotaStatus {
    pub minutely_used: u64,
    pub minutely_limit: u64,
    pub hourly_used: u64,
    pub hourly_limit: u64,
    pub daily_used: u64,
    pub daily_limit: u64,
}

/// Thread-safe API Quota Governor to proactively prevent rate-limits and unexpected API fees
pub struct ApiQuotaGovernor {
    pub name: String,
    limit_minutely: u64,
    limit_hourly: u64,
    limit_daily: u64,
    count_minutely: AtomicU64,
    count_hourly: AtomicU64,
    count_daily: AtomicU64,
    window_minute: RwLock<Instant>,
    window_hour: RwLock<Instant>,
    window_day: RwLock<Instant>,
}

impl ApiQuotaGovernor {
    pub fn new(name: &str, minutely: u64, hourly: u64, daily: u64) -> Self {
        let now = Instant::now();
        Self {
            name: name.to_string(),
            limit_minutely: minutely,
            limit_hourly: hourly,
            limit_daily: daily,
            count_minutely: AtomicU64::new(0),
            count_hourly: AtomicU64::new(0),
            count_daily: AtomicU64::new(0),
            window_minute: RwLock::new(now),
            window_hour: RwLock::new(now),
            window_day: RwLock::new(now),
        }
    }

    /// Checks whether an API call is permitted using the default English locale.
    pub fn check_and_increment(&self) -> Result<QuotaStatus, String> {
        self.check_and_increment_for_lang("en")
    }

    /// Checks whether an API call is permitted, returning localized error messages.
    /// If a limit is about to be exceeded, the request is BLOCKED proactively.
    pub fn check_and_increment_for_lang(&self, lang: &str) -> Result<QuotaStatus, String> {
        let now = Instant::now();

        // 1. Check 60-second window
        {
            let mut w = self.window_minute.write().unwrap();
            if w.elapsed() >= Duration::from_secs(60) {
                *w = now;
                self.count_minutely.store(0, Ordering::Relaxed);
            }
        }

        // 2. Check 3600-second window
        {
            let mut w = self.window_hour.write().unwrap();
            if w.elapsed() >= Duration::from_secs(3600) {
                *w = now;
                self.count_hourly.store(0, Ordering::Relaxed);
            }
        }

        // 3. Check 86400-second window
        {
            let mut w = self.window_day.write().unwrap();
            if w.elapsed() >= Duration::from_secs(86400) {
                *w = now;
                self.count_daily.store(0, Ordering::Relaxed);
            }
        }

        let m = self.count_minutely.load(Ordering::Relaxed);
        let h = self.count_hourly.load(Ordering::Relaxed);
        let d = self.count_daily.load(Ordering::Relaxed);

        if self.limit_minutely > 0 && m >= self.limit_minutely {
            warn!("[{}] Minutely limit reached: {}/{}", self.name, m, self.limit_minutely);
            let msg = if lang == "nl" {
                format!(
                    "⚠️ [{}] Minuutlimiet bereikt ({}/{} calls/min). Aanvraag tijdelijk geblokkeerd om rate-limits te voorkomen.",
                    self.name, m, self.limit_minutely
                )
            } else {
                format!(
                    "⚠️ [{}] Minutely limit reached ({}/{} calls/min). Request temporarily blocked to prevent rate-limits.",
                    self.name, m, self.limit_minutely
                )
            };
            return Err(msg);
        }

        if self.limit_hourly > 0 && h >= self.limit_hourly {
            warn!("[{}] Hourly limit reached: {}/{}", self.name, h, self.limit_hourly);
            let msg = if lang == "nl" {
                format!(
                    "⚠️ [{}] Uurlimiet bereikt ({}/{} calls/uur). Aanvraag geblokkeerd om rate-limits te voorkomen.",
                    self.name, h, self.limit_hourly
                )
            } else {
                format!(
                    "⚠️ [{}] Hourly limit reached ({}/{} calls/hour). Request blocked to prevent rate-limits.",
                    self.name, h, self.limit_hourly
                )
            };
            return Err(msg);
        }

        if self.limit_daily > 0 && d >= self.limit_daily {
            warn!("[{}] Daily limit reached: {}/{}", self.name, d, self.limit_daily);
            let msg = if lang == "nl" {
                format!(
                    "⚠️ [{}] Daglimiet bereikt ({}/{} calls/dag). Aanvraag geblokkeerd om kosten en blokkades te voorkomen.",
                    self.name, d, self.limit_daily
                )
            } else {
                format!(
                    "⚠️ [{}] Daily limit reached ({}/{} calls/day). Request blocked to prevent rate-limits.",
                    self.name, d, self.limit_daily
                )
            };
            return Err(msg);
        }

        // Increment counters
        let new_m = self.count_minutely.fetch_add(1, Ordering::Relaxed) + 1;
        let new_h = self.count_hourly.fetch_add(1, Ordering::Relaxed) + 1;
        let new_d = self.count_daily.fetch_add(1, Ordering::Relaxed) + 1;

        Ok(QuotaStatus {
            minutely_used: new_m,
            minutely_limit: self.limit_minutely,
            hourly_used: new_h,
            hourly_limit: self.limit_hourly,
            daily_used: new_d,
            daily_limit: self.limit_daily,
        })
    }

    /// Geeft de huidige tellerstanden en limieten terug
    pub fn get_status(&self) -> QuotaStatus {
        let now = Instant::now();
        {
            let mut w = self.window_minute.write().unwrap();
            if w.elapsed() >= Duration::from_secs(60) {
                *w = now;
                self.count_minutely.store(0, Ordering::Relaxed);
            }
        }
        {
            let mut w = self.window_hour.write().unwrap();
            if w.elapsed() >= Duration::from_secs(3600) {
                *w = now;
                self.count_hourly.store(0, Ordering::Relaxed);
            }
        }
        {
            let mut w = self.window_day.write().unwrap();
            if w.elapsed() >= Duration::from_secs(86400) {
                *w = now;
                self.count_daily.store(0, Ordering::Relaxed);
            }
        }

        QuotaStatus {
            minutely_used: self.count_minutely.load(Ordering::Relaxed),
            minutely_limit: self.limit_minutely,
            hourly_used: self.count_hourly.load(Ordering::Relaxed),
            hourly_limit: self.limit_hourly,
            daily_used: self.count_daily.load(Ordering::Relaxed),
            daily_limit: self.limit_daily,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quota_governor_blocking() {
        let gov = ApiQuotaGovernor::new("TestAPI", 2, 5, 10);
        assert!(gov.check_and_increment().is_ok());
        assert!(gov.check_and_increment().is_ok());
        // Third call in the same minute should be blocked
        let res_en = gov.check_and_increment();
        assert!(res_en.is_err());
        assert!(res_en.unwrap_err().contains("Minutely limit reached"));

        let res_nl = gov.check_and_increment_for_lang("nl");
        assert!(res_nl.is_err());
        assert!(res_nl.unwrap_err().contains("Minuutlimiet bereikt"));
    }

    #[test]
    fn test_quota_governor_status() {
        let gov = ApiQuotaGovernor::new("TestAPI", 10, 20, 30);
        assert!(gov.check_and_increment().is_ok());
        let status = gov.get_status();
        assert_eq!(status.minutely_used, 1);
        assert_eq!(status.hourly_used, 1);
        assert_eq!(status.daily_used, 1);
    }
}
