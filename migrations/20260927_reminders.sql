-- Herinneringen / Reminders tabel voor de actieve achtergrond-timer (!remind / !remindme)
CREATE TABLE IF NOT EXISTS reminders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    author TEXT NOT NULL,
    channel TEXT NOT NULL,
    platform TEXT NOT NULL,
    message TEXT NOT NULL,
    trigger_at INTEGER NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    delivered_at TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_reminders_trigger ON reminders(trigger_at, delivered_at);
