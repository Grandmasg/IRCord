-- Vaste weerlocatie per gebruiker (!weer set <plaatsnaam>)
CREATE TABLE IF NOT EXISTS user_weather_locations (
    platform TEXT NOT NULL,
    user_id TEXT NOT NULL,
    location TEXT NOT NULL,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (platform, user_id)
);
CREATE INDEX IF NOT EXISTS idx_user_weather_user ON user_weather_locations(user_id);
