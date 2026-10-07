use serde::Deserialize;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

#[derive(Debug, Deserialize, Default)]
struct LanguageDetectionConfig {
    #[serde(default)]
    distinct_words: Vec<String>,
    #[serde(default)]
    casual_banter: Vec<String>,
}

#[allow(dead_code)] // meta wordt alleen gedeserialiseerd
#[derive(Debug, Deserialize, Default)]
struct LocaleFile {
    #[serde(default)]
    meta: Option<HashMap<String, String>>,
    #[serde(default)]
    aliases: HashMap<String, Vec<String>>,
    #[serde(default)]
    messages: HashMap<String, String>,
    #[serde(default)]
    language_detection: Option<LanguageDetectionConfig>,
}

#[derive(Debug, Clone)]
pub struct LocaleManager {
    locales_dir: PathBuf,
    default_language: String,
    active_language: String,
    available_languages: Vec<String>,
    alias_to_canonical: HashMap<String, String>,
    all_messages: Arc<HashMap<String, HashMap<String, String>>>,
    distinct_words: Arc<HashMap<String, Vec<String>>>,
    casual_banter: Arc<Vec<String>>,
    fallback_messages: HashMap<String, String>,
    user_preferences: Arc<RwLock<HashMap<(String, String), String>>>,
}

const EMBEDDED_NL: &str = include_str!("../../locales/nl.toml");
const EMBEDDED_EN: &str = include_str!("../../locales/en.toml");
const EMBEDDED_DE: &str = include_str!("../../locales/de.toml");
const EMBEDDED_FR: &str = include_str!("../../locales/fr.toml");
const EMBEDDED_ES: &str = include_str!("../../locales/es.toml");
const EMBEDDED_ZH: &str = include_str!("../../locales/zh.toml");

impl LocaleManager {
    /// Loads all locale files from the specified directory (defaults to "locales")
    pub fn load<P: AsRef<Path>>(locales_dir: P, default_lang: &str) -> Self {
        let dir = locales_dir.as_ref().to_path_buf();
        let mut alias_to_canonical = HashMap::new();
        let mut all_messages = HashMap::new();
        let mut available_languages = Vec::new();
        let mut distinct_words = HashMap::new();
        let mut casual_banter = Vec::new();

        // 0. Pre-load embedded compile-time defaults (fail-safe for Docker containers)
        for (lang_code, content) in [
            ("nl", EMBEDDED_NL),
            ("en", EMBEDDED_EN),
            ("de", EMBEDDED_DE),
            ("fr", EMBEDDED_FR),
            ("es", EMBEDDED_ES),
            ("zh", EMBEDDED_ZH),
        ] {
            if let Ok(loc_file) = toml::from_str::<LocaleFile>(content) {
                for (canonical, aliases) in loc_file.aliases {
                    alias_to_canonical.insert(canonical.to_lowercase(), canonical.clone());
                    for alias in aliases {
                        alias_to_canonical.insert(alias.to_lowercase(), canonical.clone());
                    }
                }
                all_messages.insert(lang_code.to_string(), loc_file.messages);
                available_languages.push(lang_code.to_string());

                if let Some(ld) = loc_file.language_detection {
                    if !ld.distinct_words.is_empty() {
                        distinct_words.insert(lang_code.to_string(), ld.distinct_words);
                    }
                    for b in ld.casual_banter {
                        casual_banter.push(b.to_lowercase());
                    }
                }
            }
        }

        // 1. Scan directory for all .toml locale files (allows runtime overriding)
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let lang_code = stem.to_lowercase();
                        if let Ok(content) = fs::read_to_string(&path) {
                            match toml::from_str::<LocaleFile>(&content) {
                                Ok(loc_file) => {
                                    // Register canonical triggers and aliases
                                    for (canonical, aliases) in loc_file.aliases {
                                        alias_to_canonical.insert(canonical.to_lowercase(), canonical.clone());
                                        for alias in aliases {
                                            alias_to_canonical.insert(alias.to_lowercase(), canonical.clone());
                                        }
                                    }
                                    all_messages.insert(lang_code.clone(), loc_file.messages);
                                    available_languages.push(lang_code.clone());

                                    if let Some(ld) = loc_file.language_detection {
                                        if !ld.distinct_words.is_empty() {
                                            distinct_words.insert(lang_code.clone(), ld.distinct_words);
                                        }
                                        for b in ld.casual_banter {
                                            casual_banter.push(b.to_lowercase());
                                        }
                                    }
                                }
                                Err(e) => warn!("Error parsing locale file {:?}: {}", path, e),
                            }
                        }
                    }
                }
            }
        }

        available_languages.sort();
        available_languages.dedup();
        casual_banter.sort();
        casual_banter.dedup();

        let fallback_messages = all_messages.get("en").cloned().unwrap_or_default();
        let active_language = default_lang.to_string();

        Self {
            locales_dir: dir,
            default_language: default_lang.to_string(),
            active_language,
            available_languages,
            alias_to_canonical,
            all_messages: Arc::new(all_messages),
            distinct_words: Arc::new(distinct_words),
            casual_banter: Arc::new(casual_banter),
            fallback_messages,
            user_preferences: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Creates a lightweight view of LocaleManager for a specific active language
    pub fn for_language(&self, lang: &str) -> Self {
        Self {
            locales_dir: self.locales_dir.clone(),
            default_language: self.default_language.clone(),
            active_language: lang.to_string(),
            available_languages: self.available_languages.clone(),
            alias_to_canonical: self.alias_to_canonical.clone(),
            all_messages: Arc::clone(&self.all_messages),
            distinct_words: Arc::clone(&self.distinct_words),
            casual_banter: Arc::clone(&self.casual_banter),
            fallback_messages: self.fallback_messages.clone(),
            user_preferences: Arc::clone(&self.user_preferences),
        }
    }

    /// Returns the active language code (e.g. "nl", "de", or "en")
    pub fn language(&self) -> &str {
        &self.active_language
    }

    /// Returns the global default language code
    pub fn default_language(&self) -> &str {
        &self.default_language
    }

    /// Returns all available language codes discovered in the locales directory
    pub fn available_languages(&self) -> &[String] {
        &self.available_languages
    }

    /// Checks if the active language is Dutch
    pub fn is_dutch(&self) -> bool {
        self.active_language == "nl"
    }

    /// Returns the explicit user language preference if one has been set
    pub fn get_user_preference(&self, platform: &str, user: &str) -> Option<String> {
        let key = (platform.to_lowercase(), user.to_lowercase());
        if let Ok(lock) = self.user_preferences.read() {
            return lock.get(&key).cloned();
        }
        None
    }

    /// Resolves user language preference or falls back to default language
    pub fn user_lang(&self, platform: &str, user: &str) -> String {
        if let Some(pref) = self.get_user_preference(platform, user) {
            return pref;
        }
        self.default_language.clone()
    }

    /// Normalizes language names or codes to supported 2-letter codes
    pub fn normalize_language_code(&self, input: &str) -> Option<&str> {
        let trimmed = input.trim().to_lowercase();
        match trimmed.as_str() {
            "nl" | "nederlands" | "dutch" => Some("nl"),
            "en" | "engels" | "english" => Some("en"),
            "de" | "duits" | "deutsch" | "german" => Some("de"),
            "zh" | "chinees" | "chinese" | "中文" | "汉语" | "漢語" => Some("zh"),
            other => {
                self.available_languages.iter().find(|l| l.as_str() == other).map(|s| s.as_str())
            }
        }
    }

    /// Sets user language in memory cache
    pub fn set_user_language(&self, platform: &str, user: &str, lang: &str) {
        let key = (platform.to_lowercase(), user.to_lowercase());
        if let Ok(mut lock) = self.user_preferences.write() {
            lock.insert(key, lang.to_lowercase());
        }
    }

    /// Returns distinct stopwords for a specific language
    pub fn get_distinct_words(&self, lang: &str) -> Option<&[String]> {
        self.distinct_words.get(&lang.to_lowercase()).map(|v| v.as_slice())
    }

    /// Checks if a message consists entirely of casual banter, slang, or common IRC words
    pub fn is_casual_banter(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        let words: Vec<&str> = lower
            .split(|c: char| !c.is_alphabetic())
            .filter(|w| !w.is_empty())
            .collect();

        if words.is_empty() {
            return true;
        }

        // If banter list loaded from locales, check against that
        if !self.casual_banter.is_empty() {
            words.iter().all(|w| self.casual_banter.iter().any(|b| b == w))
        } else {
            // Built-in baseline fallback
            const DEFAULT_SLANG: &[&str] = &[
                "yeah", "yea", "yep", "nope", "yes", "no", "nah", "thanks", "thx", "ty", "pls", "please",
                "okay", "ok", "k", "cool", "nice", "shit", "fuck", "damn", "wtf", "omg", "lol", "lmao",
                "rofl", "gg", "gl", "hf", "bye", "hi", "hey", "hello", "good luck", "no problem", "np",
                "afk", "brb", "wb", "link", "update", "browser", "motherfucker", "cheers", "proost",
                "firefox", "chrome", "edge", "settings", "portainer", "docker", "server", "synology",
                "gheghe", "haha", "hahaha", "hehe", "hehehe",
            ];
            words.iter().all(|w| DEFAULT_SLANG.contains(w))
        }
    }

    /// Resets user language preference from memory cache
    pub fn reset_user_language(&self, platform: &str, user: &str) {
        let key = (platform.to_lowercase(), user.to_lowercase());
        if let Ok(mut lock) = self.user_preferences.write() {
            lock.remove(&key);
        }
    }

    /// Preloads all saved user preferences from SQLite
    pub async fn load_preferences_from_db(&self, pool: &SqlitePool) -> Result<(), sqlx::Error> {
        let rows = sqlx::query("SELECT platform, user_id, language FROM user_preferences")
            .fetch_all(pool)
            .await?;

        if let Ok(mut lock) = self.user_preferences.write() {
            for row in rows {
                let platform: String = row.get("platform");
                let user_id: String = row.get("user_id");
                let lang: String = row.get("language");
                lock.insert((platform.to_lowercase(), user_id.to_lowercase()), lang.to_lowercase());
            }
            info!("Loaded {} user language preference(s) from database.", lock.len());
        }
        Ok(())
    }

    /// Persists user language preference to SQLite and updates in-memory cache
    pub async fn persist_user_language(
        &self,
        pool: &SqlitePool,
        platform: &str,
        user: &str,
        lang: &str,
    ) -> Result<(), sqlx::Error> {
        self.set_user_language(platform, user, lang);
        sqlx::query(
            r#"
            INSERT INTO user_preferences (platform, user_id, language, updated_at)
            VALUES (?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(platform, user_id) DO UPDATE SET language = excluded.language, updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(platform.to_lowercase())
        .bind(user.to_lowercase())
        .bind(lang.to_lowercase())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Removes user language preference from SQLite and memory cache
    pub async fn remove_user_language(
        &self,
        pool: &SqlitePool,
        platform: &str,
        user: &str,
    ) -> Result<(), sqlx::Error> {
        self.reset_user_language(platform, user);
        sqlx::query("DELETE FROM user_preferences WHERE platform = ? AND user_id = ?")
            .bind(platform.to_lowercase())
            .bind(user.to_lowercase())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Resolves any localized alias to the canonical English command trigger.
    /// For example: "weer" -> "weather", "tijd" -> "time", "wetter" -> "weather".
    /// If no alias is registered, returns the input unchanged.
    pub fn resolve_alias<'a>(&'a self, input: &'a str) -> &'a str {
        let lower = input.to_lowercase();
        if let Some(canonical) = self.alias_to_canonical.get(&lower) {
            canonical.as_str()
        } else {
            input
        }
    }

    /// Retrieves a translated message string with automatic fallback to English
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        if let Some(msg) = self.all_messages.get(&self.active_language).and_then(|m| m.get(key)) {
            msg.as_str()
        } else if let Some(fallback) = self.fallback_messages.get(key) {
            fallback.as_str()
        } else {
            key
        }
    }

    /// Retrieves a translated message and replaces placeholders like {city} or {location}
    pub fn tf(&self, key: &str, replacements: &[(&str, &str)]) -> String {
        let mut text = self.t(key).to_string();
        for (placeholder, val) in replacements {
            let pattern = format!("{{{}}}", placeholder);
            text = text.replace(&pattern, val);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_locale_loads_and_resolves() {
        let loc = LocaleManager::load("locales", "zh");
        assert_eq!(loc.language(), "zh");
        assert_eq!(loc.t("weather_title"), "天气");
        assert_eq!(loc.tf("karma_score", &[("target", "x"), ("score", "3")]), "[Karma] x 的得分是 3");
    }

    /// Elke taal moet dezelfde bericht-sleutels hebben als het Engels (anders valt de gebruiker terug op het Engels).
    #[test]
    fn all_locales_have_complete_messages() {
        let load = |code: &str| -> toml::Value {
            let raw = std::fs::read_to_string(format!("locales/{}.toml", code)).unwrap();
            toml::from_str(&raw).unwrap_or_else(|e| panic!("locales/{}.toml is geen geldige TOML: {}", code, e))
        };
        let keys = |v: &toml::Value| -> std::collections::BTreeSet<String> {
            v["messages"].as_table().unwrap().keys().cloned().collect()
        };
        let en = keys(&load("en"));
        for code in ["nl", "de", "fr", "es", "zh"] {
            let lang = keys(&load(code));
            let missing: Vec<_> = en.difference(&lang).collect();
            assert!(missing.is_empty(), "{} mist vertalingen: {:?}", code, missing);
        }
    }


    #[test]
    fn test_locale_manager_dutch() {
        let mgr = LocaleManager::load("locales", "nl");
        assert_eq!(mgr.language(), "nl");
        assert!(mgr.is_dutch());

        // Alias resolution
        assert_eq!(mgr.resolve_alias("weer"), "weather");
        assert_eq!(mgr.resolve_alias("tijd"), "time");
        assert_eq!(mgr.resolve_alias("valuta"), "currency");
        assert_eq!(mgr.resolve_alias("mep"), "slap");
        assert_eq!(mgr.resolve_alias("watis"), "whatis");
        assert_eq!(mgr.resolve_alias("vertaal"), "translate");

        // Canonical commands should remain unchanged or resolve to themselves
        assert_eq!(mgr.resolve_alias("weather"), "weather");
        assert_eq!(mgr.resolve_alias("unknowncmd"), "unknowncmd");

        // Translations
        assert_eq!(mgr.t("weather_title"), "Weer");
        let formatted = mgr.tf("weather_not_found", &[("city", "Groningen")]);
        assert_eq!(formatted, "Plaats 'Groningen' niet gevonden.");
    }

    #[test]
    fn test_locale_manager_fallback() {
        let mgr = LocaleManager::load("locales", "de");
        assert_eq!(mgr.language(), "de");
        assert!(!mgr.is_dutch());

        // German alias
        assert_eq!(mgr.resolve_alias("wetter"), "weather");
        assert_eq!(mgr.resolve_alias("zeit"), "time");
        assert_eq!(mgr.resolve_alias("schlagen"), "slap");

        // Fallback for missing keys
        assert_eq!(mgr.t("unknown_key_xyz"), "unknown_key_xyz");
    }

    #[test]
    fn test_user_language_preference() {
        let mgr = LocaleManager::load("locales", "nl");
        assert_eq!(mgr.language(), "nl");

        // Default user has global default language
        assert_eq!(mgr.user_lang("irc", "Alice"), "nl");
        assert_eq!(mgr.t("weather_title"), "Weer");

        // Alice sets preference to English
        mgr.set_user_language("irc", "Alice", "en");
        assert_eq!(mgr.user_lang("irc", "Alice"), "en");

        // Scoped user manager gets English
        let alice_mgr = mgr.for_language(&mgr.user_lang("irc", "Alice"));
        assert_eq!(alice_mgr.language(), "en");
        assert_eq!(alice_mgr.t("weather_title"), "Weather");

        // Bob still gets Dutch
        assert_eq!(mgr.user_lang("irc", "Bob"), "nl");
        assert_eq!(mgr.t("weather_title"), "Weer");

        // Alice resets
        mgr.reset_user_language("irc", "Alice");
        assert_eq!(mgr.user_lang("irc", "Alice"), "nl");

        // Normalization
        assert_eq!(mgr.normalize_language_code("dutch"), Some("nl"));
        assert_eq!(mgr.normalize_language_code("EN"), Some("en"));
        assert_eq!(mgr.normalize_language_code("deutsch"), Some("de"));
        assert_eq!(mgr.normalize_language_code("invalid_lang"), None);
    }
}

