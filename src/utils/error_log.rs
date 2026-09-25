use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize)]
pub struct ErrorLogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub source: String,
    pub message: String,
}

pub struct ErrorLogger {
    capacity: usize,
    entries: RwLock<VecDeque<ErrorLogEntry>>,
}

impl ErrorLogger {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: RwLock::new(VecDeque::with_capacity(capacity)),
        }
    }

    pub fn record(&self, level: &str, source: &str, message: &str) {
        if let Ok(mut entries) = self.entries.write() {
            if entries.len() >= self.capacity {
                entries.pop_front();
            }
            entries.push_back(ErrorLogEntry {
                timestamp: Utc::now(),
                level: level.to_string(),
                source: source.to_string(),
                message: message.to_string(),
            });
        }
    }

    pub fn recent(&self, count: usize) -> Vec<ErrorLogEntry> {
        if let Ok(entries) = self.entries.read() {
            let start = if entries.len() > count {
                entries.len() - count
            } else {
                0
            };
            entries.iter().skip(start).cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn count(&self) -> usize {
        if let Ok(entries) = self.entries.read() {
            entries.len()
        } else {
            0
        }
    }

    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
    }
}
