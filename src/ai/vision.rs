use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use tracing::debug;

use super::freetoken::FreeTokenClient;

pub struct VisionHelper {
    cache: Mutex<LruCache<String, String>>, // Image URL/Hash -> Description
}

impl VisionHelper {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(LruCache::new(NonZeroUsize::new(200).unwrap())),
        }
    }

    /// Controleert of er al een beschrijving in de cache aanwezig is voor deze afbeeldings-URL
    pub fn get_cached_description(&self, url: &str) -> Option<String> {
        let mut cache = self.cache.lock().unwrap();
        cache.get(url).cloned()
    }

    pub fn cache_description(&self, url: String, desc: String) {
        let mut cache = self.cache.lock().unwrap();
        cache.put(url, desc);
    }

    /// Haalt een beschrijving op uit de cache of raadpleegt het lokale vision model
    pub async fn get_or_describe(&self, url: &str, client: &FreeTokenClient, model: Option<&str>) -> Option<String> {
        if let Some(cached) = self.get_cached_description(url) {
            return Some(cached);
        }

        debug!("Genereren AI beschrijving voor afbeelding: {}", url);
        match client.describe_image(url, model).await {
            Ok(desc) => {
                self.cache_description(url.to_string(), desc.clone());
                Some(desc)
            }
            Err(err) => {
                debug!("Kon afbeelding niet beschrijven via AI ({}): {}", url, err);
                None
            }
        }
    }

    /// Formatteert een beknopte 1-regel omschrijving voor weergave op IRC
    pub fn format_for_irc(description: &str, is_dutch: bool) -> String {
        let label = if is_dutch { "AI Afbeelding" } else { "AI Vision" };
        format!("🖼️ [{}]: {}", label, description.trim().replace('\n', " "))
    }
}
