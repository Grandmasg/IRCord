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

#[derive(Deserialize, Debug)]
struct MeteoResponse {
    current: Option<CurrentMeteo>,
    current_weather: Option<LegacyCurrentWeather>,
    daily: Option<DailyMeteo>,
}

#[derive(Deserialize, Debug)]
struct CurrentMeteo {
    temperature_2m: Option<f64>,
    relative_humidity_2m: Option<u8>,
    apparent_temperature: Option<f64>,
    precipitation: Option<f64>,
    weather_code: Option<u8>,
    wind_speed_10m: Option<f64>,
    wind_direction_10m: Option<f64>,
    surface_pressure: Option<f64>,
}

#[derive(Deserialize, Debug)]
struct LegacyCurrentWeather {
    temperature: f64,
    windspeed: f64,
}

#[derive(Deserialize, Debug)]
struct DailyMeteo {
    temperature_2m_max: Option<Vec<f64>>,
    temperature_2m_min: Option<Vec<f64>>,
    precipitation_probability_max: Option<Vec<u8>>,
    sunrise: Option<Vec<String>>,
    sunset: Option<Vec<String>>,
}

/// Vertaalt WMO weercodes naar een passende emoji en tekstomschrijving
fn wmo_code_to_desc(code: u8, is_dutch: bool) -> (&'static str, &'static str) {
    match code {
        0 => ("☀️", if is_dutch { "Helder / Zonnig" } else { "Clear sky" }),
        1 => ("🌤️", if is_dutch { "Overwegend helder" } else { "Mainly clear" }),
        2 => ("⛅", if is_dutch { "Halfbewolkt" } else { "Partly cloudy" }),
        3 => ("☁️", if is_dutch { "Zwaarbewolkt" } else { "Overcast" }),
        45 | 48 => ("🌫️", if is_dutch { "Mist / Rijp" } else { "Fog / Rime fog" }),
        51 | 53 | 55 => ("🌦️", if is_dutch { "Motregen" } else { "Drizzle" }),
        56 | 57 => ("🌧️", if is_dutch { "Aanvriezende motregen" } else { "Freezing drizzle" }),
        61 => ("🌧️", if is_dutch { "Lichte regen" } else { "Slight rain" }),
        63 => ("🌧️", if is_dutch { "Matige regen" } else { "Moderate rain" }),
        65 => ("🌧️", if is_dutch { "Zware regenval" } else { "Heavy rain" }),
        66 | 67 => ("🌨️", if is_dutch { "Aanvriezende regen" } else { "Freezing rain" }),
        71 => ("🌨️", if is_dutch { "Lichte sneeuw" } else { "Slight snow" }),
        73 => ("🌨️", if is_dutch { "Matige sneeuwval" } else { "Moderate snow" }),
        75 => ("❄️", if is_dutch { "Zware sneeuwval" } else { "Heavy snow" }),
        77 => ("❄️", if is_dutch { "Kornsneeuw" } else { "Snow grains" }),
        80..=82 => ("🌦️", if is_dutch { "Regenbuien" } else { "Rain showers" }),
        85 | 86 => ("🌨️", if is_dutch { "Sneeuwbuien" } else { "Snow showers" }),
        95 => ("⛈️", if is_dutch { "Onweersbui" } else { "Thunderstorm" }),
        96 | 99 => ("⛈️", if is_dutch { "Onweer met hagel" } else { "Thunderstorm with hail" }),
        _ => ("🌡️", if is_dutch { "Wisselvallig" } else { "Variable" }),
    }
}

/// Converteert windrichting in graden (0-360) naar een windstreek (bijv. NNO, ZW)
fn wind_dir_to_compass(deg: f64, is_dutch: bool) -> &'static str {
    let dirs_nl = ["N", "NNO", "NO", "ONO", "O", "OZO", "ZO", "ZZO", "Z", "ZZW", "ZW", "WZW", "W", "WNW", "NW", "NNW"];
    let dirs_en = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
    let idx = ((deg + 11.25) / 22.5).floor() as usize % 16;
    if is_dutch { dirs_nl[idx] } else { dirs_en[idx] }
}

/// Berekent de schaal van Beaufort op basis van wind in km/h
fn kmh_to_bft(kmh: f64) -> u8 {
    match kmh {
        k if k < 1.0 => 0,
        k if k <= 5.0 => 1,
        k if k <= 11.0 => 2,
        k if k <= 19.0 => 3,
        k if k <= 28.0 => 4,
        k if k <= 38.0 => 5,
        k if k <= 49.0 => 6,
        k if k <= 61.0 => 7,
        k if k <= 74.0 => 8,
        k if k <= 88.0 => 9,
        k if k <= 102.0 => 10,
        k if k <= 117.0 => 11,
        _ => 12,
    }
}

/// Extraheert het tijdstip "HH:MM" uit een ISO8601 string zoals "2026-09-28T07:36"
fn extract_time_hhmm(iso_str: &str) -> &str {
    if let Some(t_idx) = iso_str.find('T') {
        &iso_str[t_idx + 1..iso_str.len().min(t_idx + 6)]
    } else {
        iso_str
    }
}

pub(crate) async fn get_user_location(
    ctx: &PluginContext,
    platform: &str,
    author: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Directe lookup op author en platform (of platform='any')
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT location FROM user_weather_locations WHERE LOWER(user_id) = LOWER(?) AND (platform = ? OR platform = 'any') LIMIT 1"
    )
    .bind(author)
    .bind(platform)
    .fetch_optional(&ctx.db)
    .await?;

    if let Some(r) = row {
        return Ok(Some(r.0));
    }

    // 2. Probeer gekoppeld account (IRC <=> Discord account_links)
    let linked: Option<(String, String)> = sqlx::query_as(
        "SELECT irc_nick, discord_tag FROM account_links WHERE LOWER(irc_nick) = LOWER(?) OR LOWER(discord_tag) = LOWER(?) LIMIT 1"
    )
    .bind(author)
    .bind(author)
    .fetch_optional(&ctx.db)
    .await
    .unwrap_or(None);

    if let Some((irc_nick, discord_tag)) = linked {
        let other_id = if author.eq_ignore_ascii_case(&irc_nick) {
            discord_tag
        } else {
            irc_nick
        };
        let row_linked: Option<(String,)> = sqlx::query_as(
            "SELECT location FROM user_weather_locations WHERE LOWER(user_id) = LOWER(?) LIMIT 1"
        )
        .bind(other_id)
        .fetch_optional(&ctx.db)
        .await
        .unwrap_or(None);

        if let Some(r) = row_linked {
            return Ok(Some(r.0));
        }
    }

    Ok(None)
}

async fn set_user_location(
    ctx: &PluginContext,
    platform: &str,
    author: &str,
    location: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_weather_locations (
            platform TEXT NOT NULL,
            user_id TEXT NOT NULL,
            location TEXT NOT NULL,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (platform, user_id)
        )"
    )
    .execute(&ctx.db)
    .await;

    sqlx::query(
        "INSERT INTO user_weather_locations (platform, user_id, location, updated_at)
         VALUES (?, LOWER(?), ?, CURRENT_TIMESTAMP)
         ON CONFLICT(platform, user_id) DO UPDATE SET location = excluded.location, updated_at = CURRENT_TIMESTAMP"
    )
    .bind(platform)
    .bind(author)
    .bind(location)
    .execute(&ctx.db)
    .await?;

    Ok(())
}

async fn unset_user_location(
    ctx: &PluginContext,
    platform: &str,
    author: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    sqlx::query(
        "DELETE FROM user_weather_locations WHERE LOWER(user_id) = LOWER(?) AND (platform = ? OR platform = 'any')"
    )
    .bind(author)
    .bind(platform)
    .execute(&ctx.db)
    .await?;

    Ok(())
}

#[async_trait]
impl Plugin for WeatherPlugin {
    fn name(&self) -> &'static str { "weather" }
    fn triggers(&self) -> &[&'static str] { &["weather", "weer", "wetter"] }
    fn help(&self) -> &'static str { "!weather [<city> | set <city> | unset] / !weer [<plaatsnaam> | set <plaatsnaam> | unset] - Actueel weerbericht (of stel je vaste locatie in met 'set')" }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trimmed_args = cmd.args.trim();
        let lang = ctx.locale.language();
        let is_dutch = ctx.locale.is_dutch();

        // 1. Quota status opvragen
        if trimmed_args.eq_ignore_ascii_case("quota") || trimmed_args.eq_ignore_ascii_case("status") {
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

        // 2. Vaste weerlocatie wissen (!weer unset / clear / reset)
        if trimmed_args.eq_ignore_ascii_case("unset")
            || trimmed_args.eq_ignore_ascii_case("clear")
            || trimmed_args.eq_ignore_ascii_case("reset")
            || trimmed_args.eq_ignore_ascii_case("verwijder")
            || trimmed_args.eq_ignore_ascii_case("delete")
        {
            unset_user_location(ctx, &cmd.platform, &cmd.author).await?;
            return Ok(Some(ctx.locale.tf("weather_unset_success", &[("author", &cmd.author)])));
        }

        // 3. Vaste weerlocatie instellen (!weer set <plaatsnaam>)
        if trimmed_args.eq_ignore_ascii_case("set")
            || trimmed_args.eq_ignore_ascii_case("opslaan")
            || trimmed_args.eq_ignore_ascii_case("default")
        {
            return Ok(Some(ctx.locale.t("weather_set_usage").into()));
        }

        if let Some(target_loc) = trimmed_args.strip_prefix("set ")
            .or_else(|| trimmed_args.strip_prefix("Set "))
            .or_else(|| trimmed_args.strip_prefix("opslaan "))
            .or_else(|| trimmed_args.strip_prefix("Opslaan "))
            .or_else(|| trimmed_args.strip_prefix("default "))
            .or_else(|| trimmed_args.strip_prefix("Default "))
        {
            let target_loc = target_loc.trim();
            if target_loc.is_empty() {
                return Ok(Some(ctx.locale.t("weather_set_usage").into()));
            }

            // Valideer via geocoding of de locatie bestaat en haal mooie naam op
            let api_key = std::env::var("OPEN_METEO_API_KEY").ok().filter(|k| !k.trim().is_empty());
            let geo_host = if api_key.is_some() {
                "customer-geocoding-api.open-meteo.com"
            } else {
                "geocoding-api.open-meteo.com"
            };

            let mut req = ctx.http
                .get(format!("https://{}/v1/search", geo_host))
                .query(&[("name", target_loc), ("count", "1")]);
            if let Some(ref k) = api_key {
                req = req.query(&[("apikey", k.as_str())]);
            }

            let canonical_name = match req.header("User-Agent", "IRCordBot/1.0 (weather client)").send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(geo) = resp.json::<GeoResult>().await {
                        if let Some(locs) = geo.results {
                            if let Some(first) = locs.first() {
                                let country = first.country.as_deref().unwrap_or("");
                                if country.is_empty() {
                                    first.name.clone()
                                } else {
                                    format!("{}, {}", first.name, country)
                                }
                            } else {
                                return Ok(Some(ctx.locale.tf("weather_not_found", &[("city", target_loc)])));
                            }
                        } else {
                            return Ok(Some(ctx.locale.tf("weather_not_found", &[("city", target_loc)])));
                        }
                    } else {
                        target_loc.to_string()
                    }
                }
                _ => target_loc.to_string(),
            };

            set_user_location(ctx, &cmd.platform, &cmd.author, &canonical_name).await?;
            return Ok(Some(ctx.locale.tf(
                "weather_set_success",
                &[("author", &cmd.author), ("city", &canonical_name)],
            )));
        }

        // 4. Bepaal doelstad (ingevoerd of opgeslagen vaste locatie van de gebruiker)
        let (city, is_default_for_user) = if trimmed_args.is_empty() {
            if let Some(saved) = get_user_location(ctx, &cmd.platform, &cmd.author).await? {
                (saved, true)
            } else {
                return Ok(Some(ctx.locale.t("weather_usage").into()));
            }
        } else {
            (trimmed_args.to_string(), false)
        };

        let cache_key = format!(
            "{}:{}:{}:{}",
            cmd.platform,
            lang,
            city.to_lowercase(),
            if is_default_for_user { &cmd.author } else { "" }
        );
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
            let msg = if is_dutch {
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
                // 2. Weather forecast met uitgebreide meetwaarden & dagstatistieken
                let base_host = if api_key.is_some() {
                    "customer-api.open-meteo.com"
                } else {
                    "api.open-meteo.com"
                };

                let meteo_url = format!(
                    "https://{}/v1/forecast?latitude={}&longitude={}&current=temperature_2m,relative_humidity_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m,wind_direction_10m,surface_pressure&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset&timezone=auto&forecast_days=1{}",
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
                    let msg = if is_dutch {
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
                let country = loc.country.clone().unwrap_or_default();
                let title = ctx.locale.t("weather_title");

                // Extract actuele waarden
                let (temp, feels_like, humidity, precip, wmo_code, wind_speed, wind_dir, pressure) =
                    if let Some(c) = meteo.current {
                        (
                            c.temperature_2m.unwrap_or(0.0),
                            c.apparent_temperature.unwrap_or(c.temperature_2m.unwrap_or(0.0)),
                            c.relative_humidity_2m.unwrap_or(0),
                            c.precipitation.unwrap_or(0.0),
                            c.weather_code.unwrap_or(0),
                            c.wind_speed_10m.unwrap_or(0.0),
                            c.wind_direction_10m.unwrap_or(0.0),
                            c.surface_pressure.unwrap_or(1013.0),
                        )
                    } else if let Some(lw) = meteo.current_weather {
                        (lw.temperature, lw.temperature, 0, 0.0, 0, lw.windspeed, 0.0, 1013.0)
                    } else {
                        (0.0, 0.0, 0, 0.0, 0, 0.0, 0.0, 1013.0)
                    };

                let (w_emoji, w_desc) = wmo_code_to_desc(wmo_code, is_dutch);
                let wind_compass = wind_dir_to_compass(wind_dir, is_dutch);
                let bft = kmh_to_bft(wind_speed);

                // Extract dagstatistieken (min, max, regenkans, zonopkomst/ondergang)
                let min_temp = meteo.daily.as_ref().and_then(|d| d.temperature_2m_min.as_ref()).and_then(|v| v.first().copied());
                let max_temp = meteo.daily.as_ref().and_then(|d| d.temperature_2m_max.as_ref()).and_then(|v| v.first().copied());
                let rain_chance = meteo.daily.as_ref().and_then(|d| d.precipitation_probability_max.as_ref()).and_then(|v| v.first().copied());
                let sunrise = meteo.daily.as_ref().and_then(|d| d.sunrise.as_ref()).and_then(|v| v.first().map(|s| extract_time_hhmm(s)));
                let sunset = meteo.daily.as_ref().and_then(|d| d.sunset.as_ref()).and_then(|v| v.first().map(|s| extract_time_hhmm(s)));

                let min_max_str = match (min_temp, max_temp) {
                    (Some(min), Some(max)) => format!(" (min {:.1}° / max {:.1}°)", min, max),
                    _ => String::new(),
                };

                let rain_prob_str = match rain_chance {
                    Some(prob) => format!(" ({}% kans)", prob),
                    None => String::new(),
                };

                let sun_str = match (sunrise, sunset) {
                    (Some(sr), Some(ss)) => format!(" | Zon: {} - {}", sr, ss),
                    _ => String::new(),
                };

                let default_badge = if is_default_for_user {
                    format!(" • {}", ctx.locale.tf("weather_default_badge", &[("author", &cmd.author)]))
                } else {
                    String::new()
                };

                let output = if cmd.platform == "discord" {
                    format!(
                        "🌦️ **[{} {} ({}){}]** {} **{}**\n\
                        🌡️ **Temperatuur:** {:.1}°C (Gevoel {:.1}°C{}) | 💧 **Luchtvochtigheid:** {}%\n\
                        🌧️ **Neerslag:** {:.1} mm{} | 💨 **Wind:** {:.1} km/h {} ({} Bft)\n\
                        🧭 **Luchtdruk:** {:.0} hPa{}",
                        title, loc.name, country, default_badge, w_emoji, w_desc,
                        temp, feels_like, min_max_str, humidity,
                        precip, rain_prob_str, wind_speed, wind_compass, bft,
                        pressure, sun_str.replace(" | Zon:", "🌅 **Zon:**")
                    )
                } else {
                    format!(
                        "🌦️ [{} {} ({}){}] {} {} | Temp: {:.1}°C (gevoel {:.1}°C{}) | Vocht: {}% | Regen: {:.1} mm{} | Wind: {:.1} km/h {} ({} Bft) | Druk: {:.0} hPa{}",
                        title, loc.name, country, default_badge, w_emoji, w_desc,
                        temp, feels_like, min_max_str, humidity,
                        precip, rain_prob_str, wind_speed, wind_compass, bft,
                        pressure, sun_str
                    )
                };

                if let Ok(mut guard) = self.cache.lock() {
                    guard.insert(cache_key, (output.clone(), Instant::now()));
                }

                return Ok(Some(output));
            }
        }

        let not_found = ctx.locale.tf("weather_not_found", &[("city", &city)]);
        Ok(Some(not_found))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wmo_code_mapping() {
        let (e0, d0) = wmo_code_to_desc(0, true);
        assert_eq!(e0, "☀️");
        assert!(d0.contains("Helder"));

        let (e65, d65) = wmo_code_to_desc(65, true);
        assert_eq!(e65, "🌧️");
        assert!(d65.contains("Zware regen"));

        let (e75, _) = wmo_code_to_desc(75, false);
        assert_eq!(e75, "❄️");
    }

    #[test]
    fn test_wind_compass_calculation() {
        assert_eq!(wind_dir_to_compass(0.0, true), "N");
        assert_eq!(wind_dir_to_compass(90.0, true), "O");
        assert_eq!(wind_dir_to_compass(180.0, true), "Z");
        assert_eq!(wind_dir_to_compass(270.0, true), "W");
        assert_eq!(wind_dir_to_compass(45.0, true), "NO");
    }

    #[test]
    fn test_bft_calculation() {
        assert_eq!(kmh_to_bft(0.5), 0);
        assert_eq!(kmh_to_bft(10.0), 2);
        assert_eq!(kmh_to_bft(25.0), 4);
        assert_eq!(kmh_to_bft(70.0), 8);
        assert_eq!(kmh_to_bft(120.0), 12);
    }

    #[test]
    fn test_extract_time_hhmm() {
        assert_eq!(extract_time_hhmm("2026-09-28T07:36"), "07:36");
        assert_eq!(extract_time_hhmm("19:24"), "19:24");
    }

    #[tokio::test]
    async fn test_user_weather_location_persistence() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();

        let _ = sqlx::query(
            "CREATE TABLE IF NOT EXISTS user_weather_locations (
                platform TEXT NOT NULL,
                user_id TEXT NOT NULL,
                location TEXT NOT NULL,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (platform, user_id)
            )"
        )
        .execute(&pool)
        .await
        .unwrap();

        // 1. Invoegen
        sqlx::query(
            "INSERT INTO user_weather_locations (platform, user_id, location, updated_at)
             VALUES (?, LOWER(?), ?, CURRENT_TIMESTAMP)
             ON CONFLICT(platform, user_id) DO UPDATE SET location = excluded.location, updated_at = CURRENT_TIMESTAMP"
        )
        .bind("irc")
        .bind("Grandmasg")
        .bind("Amsterdam, Netherlands")
        .execute(&pool)
        .await
        .unwrap();

        // 2. Opvragen (case-insensitief)
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT location FROM user_weather_locations WHERE LOWER(user_id) = LOWER(?) AND (platform = ? OR platform = 'any') LIMIT 1"
        )
        .bind("grandmasg")
        .bind("irc")
        .fetch_optional(&pool)
        .await
        .unwrap();

        assert_eq!(row.map(|r| r.0), Some("Amsterdam, Netherlands".to_string()));

        // 3. Verwijderen
        sqlx::query(
            "DELETE FROM user_weather_locations WHERE LOWER(user_id) = LOWER(?) AND (platform = ? OR platform = 'any')"
        )
        .bind("Grandmasg")
        .bind("irc")
        .execute(&pool)
        .await
        .unwrap();

        let row_after: Option<(String,)> = sqlx::query_as(
            "SELECT location FROM user_weather_locations WHERE LOWER(user_id) = LOWER(?) AND (platform = ? OR platform = 'any') LIMIT 1"
        )
        .bind("grandmasg")
        .bind("irc")
        .fetch_optional(&pool)
        .await
        .unwrap();

        assert_eq!(row_after, None);
    }
}
