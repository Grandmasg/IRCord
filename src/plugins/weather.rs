use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct WeatherPlugin {
    cache: Mutex<HashMap<String, (String, Instant)>>,
}

impl WeatherPlugin {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for WeatherPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
struct GeoResult {
    results: Option<Vec<GeoLocation>>,
}

#[derive(Deserialize)]
struct GeoLocation {
    latitude: f64,
    longitude: f64,
    name: String,
    country: Option<String>,
}

#[derive(Deserialize)]
struct MeteoResponse {
    current_weather: CurrentWeather,
}

#[derive(Deserialize)]
struct CurrentWeather {
    temperature: f64,
    windspeed: f64,
}

#[async_trait]
impl Plugin for WeatherPlugin {
    fn name(&self) -> &'static str { "weather" }
    fn triggers(&self) -> &[&'static str] { &["weather", "weer", "wetter"] }
    fn help(&self) -> &'static str { "!weather <city> / !weer <plaatsnaam> - Current temperature and wind speed" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let city = cmd.args.trim();
        let lang = ctx.locale.language();

        if city.is_empty() {
            return Ok(Some(ctx.locale.t("weather_usage").into()));
        }

        if city.eq_ignore_ascii_case("quota") || city.eq_ignore_ascii_case("status") {
            let q = ctx.open_meteo_quota.get_status();
            let label = format!(
                "📊 [{}] {}: {}/{} | {}: {}/{} | {}: {}/{} calls",
                ctx.locale.t("weather_quota_label"),
                ctx.locale.t("weather_minute"), q.minutely_used, q.minutely_limit,
                ctx.locale.t("weather_hour"), q.hourly_used, q.hourly_limit,
                ctx.locale.t("weather_today"), q.daily_used, q.daily_limit
            );
            return Ok(Some(label));
        }

        let cache_key = format!("{}:{}", lang, city.to_lowercase());
        {
            if let Ok(guard) = self.cache.lock() {
                if let Some((cached_text, timestamp)) = guard.get(&cache_key) {
                    if timestamp.elapsed() < Duration::from_secs(600) {
                        return Ok(Some(cached_text.clone()));
                    }
                }
            }
        }

        // Proactive quota protection (blocks if API limits are about to be exceeded)
        if let Err(block_msg) = ctx.open_meteo_quota.check_and_increment_for_lang(lang) {
            return Ok(Some(block_msg));
        }

        // 1. Geocoding via Open-Meteo
        let api_key = std::env::var("OPEN_METEO_API_KEY").ok().filter(|k| !k.trim().is_empty());
        let key_param = api_key.as_ref().map(|k| format!("&apikey={}", k.trim())).unwrap_or_default();

        let geo_host = if api_key.is_some() {
            "customer-geocoding-api.open-meteo.com"
        } else {
            "geocoding-api.open-meteo.com"
        };

        let geo_url = format!(
            "https://{}/v1/search?name={}&count=1{}",
            geo_host, city, key_param
        );
        let geo_resp = ctx
            .http
            .get(&geo_url)
            .header("User-Agent", "IRCordBot/1.0 (weather client)")
            .send()
            .await?;

        if !geo_resp.status().is_success() {
            let status = geo_resp.status().as_u16();
            let msg = if ctx.locale.is_dutch() {
                if status == 429 {
                    "⚠️ Open-Meteo rate-limit bereikt. Configureer een OPEN_METEO_API_KEY in .env om limieten te verhogen."
                } else if status == 401 || status == 403 {
                    "⚠️ Ongeldige of ongeautoriseerde OPEN_METEO_API_KEY in .env."
                } else {
                    "⚠️ Kon Open-Meteo geocoding service niet bereiken."
                }
            } else {
                if status == 429 {
                    "⚠️ Open-Meteo rate limit exceeded. Configure OPEN_METEO_API_KEY in .env to increase limits."
                } else if status == 401 || status == 403 {
                    "⚠️ Invalid or unauthorized OPEN_METEO_API_KEY in .env."
                } else {
                    "⚠️ Could not reach Open-Meteo geocoding service."
                }
            };
            return Ok(Some(msg.into()));
        }

        let geo: GeoResult = geo_resp.json().await?;

        if let Some(locs) = geo.results {
            if let Some(loc) = locs.first() {
                // 2. Weather forecast
                let base_host = if api_key.is_some() {
                    "customer-api.open-meteo.com"
                } else {
                    "api.open-meteo.com"
                };

                let meteo_url = format!(
                    "https://{}/v1/forecast?latitude={}&longitude={}&current_weather=true{}",
                    base_host, loc.latitude, loc.longitude, key_param
                );
                let meteo_resp = ctx
                    .http
                    .get(&meteo_url)
                    .header("User-Agent", "IRCordBot/1.0 (weather client)")
                    .send()
                    .await?;

                if !meteo_resp.status().is_success() {
                    let status = meteo_resp.status().as_u16();
                    let msg = if ctx.locale.is_dutch() {
                        if status == 429 {
                            "⚠️ Open-Meteo rate-limit bereikt. Configureer een OPEN_METEO_API_KEY in .env."
                        } else {
                            "⚠️ Kon weersgegevens niet ophalen van Open-Meteo."
                        }
                    } else {
                        if status == 429 {
                            "⚠️ Open-Meteo rate limit exceeded. Configure OPEN_METEO_API_KEY in .env."
                        } else {
                            "⚠️ Could not retrieve weather data from Open-Meteo."
                        }
                    };
                    return Ok(Some(msg.into()));
                }

                let meteo: MeteoResponse = meteo_resp.json().await?;
                let c = meteo.current_weather;
                let country = loc.country.clone().unwrap_or_default();

                let title = ctx.locale.t("weather_title");
                let temp_label = ctx.locale.t("weather_temp");
                let wind_label = ctx.locale.t("weather_wind");

                let output = format!(
                    "🌦️ [{} {} ({})] {}: {:.1}°C | {}: {:.1} km/h",
                    title, loc.name, country, temp_label, c.temperature, wind_label, c.windspeed
                );

                if let Ok(mut guard) = self.cache.lock() {
                    guard.insert(cache_key, (output.clone(), Instant::now()));
                }

                return Ok(Some(output));
            }
        }

        let not_found = ctx.locale.tf("weather_not_found", &[("city", city)]);
        Ok(Some(not_found))
    }
}
