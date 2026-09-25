use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, info, warn};

#[derive(Debug, Deserialize, Default)]
struct LocaleFile {
    #[serde(default)]
    aliases: HashMap<String, Vec<String>>,
    #[serde(default)]
    messages: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct LocaleManager {
    language: String,
    alias_to_canonical: HashMap<String, String>,
    messages: HashMap<String, String>,
    fallback_messages: HashMap<String, String>,
}

impl LocaleManager {
    /// Laadt taalbestanden uit de opgegeven map (standaard "locales")
    pub fn load<P: AsRef<Path>>(locales_dir: P, lang: &str) -> Self {
        let dir = locales_dir.as_ref();
        let en_path = dir.join("en.toml");

        // 1. Laad altijd Engels als basis en fallback
        let en_file: LocaleFile = if en_path.exists() {
            match fs::read_to_string(&en_path) {
                Ok(content) => toml::from_str(&content).unwrap_or_default(),
                Err(e) => {
                    warn!("Kon en.toml niet lezen: {}", e);
                    LocaleFile::default()
                }
            }
        } else {
            warn!("Fallback locale bestand {:?} niet gevonden!", en_path);
            LocaleFile::default()
        };

        let mut alias_to_canonical = HashMap::new();

        // Voeg Engelse triggers en aliassen toe
        for (canonical, aliases) in &en_file.aliases {
            alias_to_canonical.insert(canonical.to_lowercase(), canonical.clone());
            for alias in aliases {
                alias_to_canonical.insert(alias.to_lowercase(), canonical.clone());
            }
        }

        let fallback_messages = en_file.messages;

        // 2. Laad de gekozen doeltaal (bijv. "nl") indien niet "en"
        let mut active_messages = fallback_messages.clone();

        if lang != "en" {
            let target_path = dir.join(format!("{}.toml", lang));
            if target_path.exists() {
                match fs::read_to_string(&target_path) {
                    Ok(content) => match toml::from_str::<LocaleFile>(&content) {
                        Ok(target_file) => {
                            info!("Taalbestand geladen voor '{}' ({:?})", lang, target_path);

                            // Registreer doeltaal aliassen
                            for (canonical, aliases) in target_file.aliases {
                                for alias in aliases {
                                    alias_to_canonical.insert(alias.to_lowercase(), canonical.clone());
                                }
                            }

                            // Overschrijf berichten met vertalingen
                            for (key, msg) in target_file.messages {
                                active_messages.insert(key, msg);
                            }
                        }
                        Err(e) => warn!("Fout bij parsen van {:?}: {}", target_path, e),
                    },
                    Err(e) => warn!("Kon {:?} niet lezen: {}", target_path, e),
                }
            } else {
                debug!("Geen specifiek taalbestand gevonden voor '{}', Engels blijft actief.", lang);
            }
        }

        Self {
            language: lang.to_string(),
            alias_to_canonical,
            messages: active_messages,
            fallback_messages,
        }
    }

    /// Geeft de actieve taalcode terug (bijv. "nl" of "en")
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Is de actieve taal Nederlands?
    pub fn is_dutch(&self) -> bool {
        self.language == "nl"
    }

    /// Vertaalt een willekeurige alias naar het canonieke Engelse commando.
    /// Bijvoorbeeld: "weer" -> "weather", "tijd" -> "time", "wetter" -> "weather".
    /// Als er geen alias bekend is, wordt de invoer ongewijzigd geretourneerd.
    pub fn resolve_alias<'a>(&'a self, input: &'a str) -> &'a str {
        let lower = input.to_lowercase();
        if let Some(canonical) = self.alias_to_canonical.get(&lower) {
            canonical.as_str()
        } else {
            input
        }
    }

    /// Haalt een vertaald tekstbericht op met automatische fallback
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        if let Some(msg) = self.messages.get(key) {
            msg.as_str()
        } else if let Some(fallback) = self.fallback_messages.get(key) {
            fallback.as_str()
        } else {
            key
        }
    }

    /// Haalt een bericht op en vervangt placeholders zoals {city} of {location}
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
}
