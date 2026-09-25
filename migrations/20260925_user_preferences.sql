-- Persoonlijke gebruikersvoorkeuren (bijv. taalvoorkeur per gebruiker)
CREATE TABLE IF NOT EXISTS user_preferences (
    platform TEXT NOT NULL,
    user_id TEXT NOT NULL,
    language TEXT NOT NULL,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (platform, user_id)
);
