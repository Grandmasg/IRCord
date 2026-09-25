use serde::Deserialize;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

#[derive(Debug, Deserialize, Default)]
struct LocaleFile {
    #[serde(default)]
    meta: Option<HashMap<String, String>>,
    #[serde(default)]
    aliases: HashMap<String, Vec<String>>,
    #[serde(default)]
    messages: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct LocaleManager {
    locales_dir: PathBuf,
    default_language: String,
    active_language: String,
    available_languages: Vec<String>,
    alias_to_canonical: HashMap<String, String>,
    all_messages: Arc<HashMap<String, HashMap<String, String>>>,
    fallback_messages: HashMap<String, String>,
    user_preferences: Arc<RwLock<HashMap<(String, String), String>>>,
}

impl LocaleManager {
    /// Loads all locale files from the specified directory (defaults to "locales")
    pub fn load<P: AsRef<Path>>(locales_dir: P, default_lang: &str) -> Self {
        let dir = locales_dir.as_ref().to_path_buf();
        let mut alias_to_canonical = HashMap::new();
        let mut all_messages = HashMap::new();
        let mut available_languages = Vec::new();

        // 1. Scan directory for all .toml locale files
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
                                    available_languages.push(lang_code);
                                }
                                Err(e) => warn!("Error parsing locale file {:?}: {}", path, e),
                            }
                        }
                    }
                }
            }
        }

        available_languages.sort();

        let fallback_messages = all_messages.get("en").cloned().unwrap_or_default();
        let active_language = default_lang.to_string();

        Self {
            locales_dir: dir,
            default_language: default_lang.to_string(),
            active_language,
            available_languages,
            alias_to_canonical,
            all_messages: Arc::new(all_messages),
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

