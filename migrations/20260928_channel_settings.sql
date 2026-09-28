-- Kanaalinstellingen voor standaard chattaal en automatische real-time vertaling (!chatlang / !autotr)
CREATE TABLE IF NOT EXISTS channel_settings (
    channel TEXT PRIMARY KEY,
    language TEXT NOT NULL,
    auto_translate BOOLEAN NOT NULL DEFAULT 0,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
