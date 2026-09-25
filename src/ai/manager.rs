use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::info;

pub struct AiManager {
    current_model: RwLock<String>,
    hourly_token_budget: u64,
    tokens_consumed: AtomicU64,
    budget_window_start: RwLock<Instant>,
}

impl AiManager {
    pub fn new(default_model: String, hourly_token_budget: u64) -> Self {
        Self {
            current_model: RwLock::new(default_model),
            hourly_token_budget,
            tokens_consumed: AtomicU64::new(0),
            budget_window_start: RwLock::new(Instant::now()),
        }
    }

    pub fn get_model(&self) -> String {
        self.current_model.read().unwrap().clone()
    }

    pub fn set_model(&self, new_model: String) {
        let mut model_lock = self.current_model.write().unwrap();
        info!("AI model gewijzigd van '{}' naar '{}'", *model_lock, new_model);
        *model_lock = new_model;
    }

    /// Controleert of het tokenbudget voor het lopende uur niet overschreden is
    pub fn can_consume(&self, estimated_tokens: u64) -> bool {
        let mut window = self.budget_window_start.write().unwrap();
        if window.elapsed() >= Duration::from_secs(3600) {
            *window = Instant::now();
            self.tokens_consumed.store(0, Ordering::Relaxed);
        }

        let current = self.tokens_consumed.load(Ordering::Relaxed);
        current + estimated_tokens <= self.hourly_token_budget
    }

    pub fn record_consumption(&self, tokens: u64) {
        self.tokens_consumed.fetch_add(tokens, Ordering::Relaxed);
    }
}
