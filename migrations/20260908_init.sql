-- Gebruikersaanwezigheid en last-seen logging
CREATE TABLE IF NOT EXISTS presence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nick TEXT NOT NULL,
    platform TEXT NOT NULL, -- 'irc' of 'discord'
    last_seen_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_spoke_at TIMESTAMP,
    last_event TEXT,        -- 'join', 'part', 'quit', 'msg'
    quit_message TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_presence_nick_platform ON presence(nick, platform);

-- WhatPulse Nickname Koppelingen
CREATE TABLE IF NOT EXISTS whatpulse_links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nick TEXT NOT NULL,
    platform TEXT NOT NULL,
    whatpulse_username TEXT NOT NULL,
    linked_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_wp_nick_platform ON whatpulse_links(nick, platform);

-- Offline Memos (!tell)
CREATE TABLE IF NOT EXISTS memos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recipient TEXT NOT NULL,
    sender TEXT NOT NULL,
    platform TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    delivered_at TIMESTAMP
);

-- Persoonlijke en Publieke RSS Feeds
CREATE TABLE IF NOT EXISTS feeds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT UNIQUE NOT NULL,
    title TEXT,
    last_guid TEXT,
    last_checked_at TIMESTAMP
);

CREATE TABLE IF NOT EXISTS feed_subscriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    feed_id INTEGER REFERENCES feeds(id) ON DELETE CASCADE,
    target_type TEXT NOT NULL, -- 'channel' of 'user_dm'
    target_id TEXT NOT NULL,   -- kanaalnaam of user nick/ID
    platform TEXT NOT NULL,    -- 'irc' of 'discord'
    keyword_filter TEXT        -- optioneel trefwoordfilter
);

-- Persoonlijke Trefwoord-Trackers (!track)
CREATE TABLE IF NOT EXISTS user_tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    platform TEXT NOT NULL,
    keyword TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Audit Trail voor Moderatie & Beheer
CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operator TEXT NOT NULL,
    platform TEXT NOT NULL,
    action TEXT NOT NULL,
    details TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Custom Aliases (!alias)
CREATE TABLE IF NOT EXISTS aliases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trigger TEXT UNIQUE NOT NULL,
    response TEXT NOT NULL,
    creator TEXT NOT NULL,
    platform TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Kanaal-Peilingen / Polls (!poll)
CREATE TABLE IF NOT EXISTS polls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel TEXT NOT NULL,
    question TEXT NOT NULL,
    options_json TEXT NOT NULL, -- JSON array van keuzes
    is_active BOOLEAN DEFAULT TRUE,
    created_by TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS poll_votes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    poll_id INTEGER REFERENCES polls(id) ON DELETE CASCADE,
    voter TEXT NOT NULL,
    platform TEXT NOT NULL,
    option_index INTEGER NOT NULL,
    voted_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(poll_id, voter, platform)
);

-- Kanaaltopic Geschiedenis
CREATE TABLE IF NOT EXISTS topic_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel TEXT NOT NULL,
    topic TEXT NOT NULL,
    set_by TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Chatlog met FTS5 voor AI-RAG
CREATE VIRTUAL TABLE IF NOT EXISTS chat_history USING fts5(
    channel,
    author,
    platform,
    message,
    timestamp
);

-- Cross-Platform Account & Nick Koppelingen (IRC <=> Discord)
CREATE TABLE IF NOT EXISTS account_links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    discord_id TEXT UNIQUE NOT NULL,      -- Discord Snowflake ID
    discord_tag TEXT NOT NULL,            -- Discord username / handle
    irc_nick TEXT UNIQUE NOT NULL,        -- Gekoppelde IRC-nick
    irc_account TEXT,                     -- Optioneel NickServ account
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_account_links_discord ON account_links(discord_id);
CREATE INDEX IF NOT EXISTS idx_account_links_irc ON account_links(irc_nick);

-- Verjaardagen Registratie & Felicitaties (!bday)
CREATE TABLE IF NOT EXISTS birthdays (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,                -- IRC nick of Discord Snowflake ID
    platform TEXT NOT NULL,               -- 'irc' of 'discord'
    display_name TEXT NOT NULL,           -- Weergavenaam voor felicitatie
    day INTEGER NOT NULL,                 -- 1 - 31
    month INTEGER NOT NULL,               -- 1 - 12
    year INTEGER,                         -- Optioneel geboortejaar
    channel TEXT NOT NULL,                -- Voorkeurskanaal
    last_celebrated_year INTEGER,         -- Voorkomt dubbele felicitaties in hetzelfde jaar
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, platform)
);
CREATE INDEX IF NOT EXISTS idx_birthdays_date ON birthdays(month, day);
