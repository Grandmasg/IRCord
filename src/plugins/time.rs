use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;

pub struct TimePlugin;

#[derive(Deserialize)]
struct GeoResult {
    results: Option<Vec<GeoLocation>>,
}

#[derive(Deserialize)]
struct GeoLocation {
    name: String,
    country: Option<String>,
    timezone: Option<String>,
}

#[derive(Deserialize)]
struct TimeApiResponse {
    date: Option<String>,
    time: Option<String>,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
    #[serde(rename = "dayOfWeek")]
    day_of_week: Option<String>,
}

#[derive(Deserialize)]
struct WorldTimeResponse {
    datetime: Option<String>,
    utc_offset: Option<String>,
    timezone: Option<String>,
}

#[async_trait]
impl Plugin for TimePlugin {
    fn name(&self) -> &'static str {
        "time"
    }

    fn triggers(&self) -> &[&'static str] {
        &["time", "tijd", "clock", "klok"]
    }

    fn help(&self) -> &'static str {
        "!time <city/country> / !tijd <stad/land> - Shows local time and timezone"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let location = cmd.args.trim();

        if location.is_empty() {
            return Ok(Some(ctx.locale.t("time_usage").into()));
        }

        // 1. Check quota & fetch location and timezone via Open-Meteo Geocoding API
        if let Err(block_msg) = ctx.open_meteo_quota.check_and_increment_for_lang(ctx.locale.language()) {
            return Ok(Some(block_msg));
        }

        let api_key = std::env::var("OPEN_METEO_API_KEY").ok().filter(|k| !k.trim().is_empty());
        let key_param = api_key.as_ref().map(|k| format!("&apikey={}", k.trim())).unwrap_or_default();
        let geo_host = if api_key.is_some() {
            "customer-geocoding-api.open-meteo.com"
        } else {
            "geocoding-api.open-meteo.com"
        };

        let geo_url = format!(
            "https://{}/v1/search?name={}&count=1{}",
            geo_host,
            urlencoding_simple(location),
            key_param
        );

        let geo_resp = ctx
            .http
            .get(&geo_url)
            .header("User-Agent", "IRCordBot/1.0 (time client)")
            .send()
            .await?;
        if !geo_resp.status().is_success() {
            return Ok(Some(ctx.locale.tf("time_not_found", &[("location", location)])));
        }

        let geo_data: GeoResult = geo_resp.json().await?;
        let loc = match geo_data.results.and_then(|r| r.into_iter().next()) {
            Some(l) => l,
            None => return Ok(Some(ctx.locale.tf("time_not_found", &[("location", location)]))),
        };

        let tz = match loc.timezone {
            Some(ref t) if !t.is_empty() => t.clone(),
            _ => return Ok(Some(ctx.locale.tf("time_no_tz", &[("location", &loc.name)]))),
        };

        let country = loc.country.unwrap_or_default();
        let country_suffix = if !country.is_empty() {
            format!(", {}", country)
        } else {
            String::new()
        };

        let title_label = ctx.locale.t("time_title");
        let hour_suffix = ctx.locale.t("time_hour_suffix");
        let tz_label = ctx.locale.t("time_timezone");
        let lang = ctx.locale.language();

        // 2. Try TimeAPI.io
        let time_url = format!("https://timeapi.io/api/time/current/zone?timeZone={}", tz);
        if let Ok(resp) = ctx.http.get(&time_url).send().await {
            if resp.status().is_success() {
                if let Ok(t_data) = resp.json::<TimeApiResponse>().await {
                    let time_str = t_data.time.unwrap_or_else(|| "??:??".to_string());
                    let date_str = t_data.date.unwrap_or_default();
                    let day_str = translate_day(t_data.day_of_week.as_deref().unwrap_or(""), lang);

                    return Ok(Some(format!(
                        "🕒 [{} {}{}] {}{} | {} {} ({})",
                        title_label, loc.name, country_suffix, time_str, hour_suffix, day_str, date_str, tz
                    )));
                }
            }
        }

        // 3. Fallback to WorldTimeAPI
        let wt_url = format!("https://worldtimeapi.org/api/timezone/{}", tz);
        if let Ok(resp) = ctx.http.get(&wt_url).send().await {
            if resp.status().is_success() {
                if let Ok(wt_data) = resp.json::<WorldTimeResponse>().await {
                    if let Some(dt) = wt_data.datetime {
                        let time_part = dt.split('T').nth(1).and_then(|t| t.split('.').next()).unwrap_or("??:??");
                        let offset = wt_data.utc_offset.unwrap_or_default();
                        return Ok(Some(format!(
                            "🕒 [{} {}{}] {} (UTC{}) | {}: {}",
                            title_label, loc.name, country_suffix, time_part, offset, tz_label, tz
                        )));
                    }
                }
            }
        }

        let loc_display = format!("{}{}", loc.name, country_suffix);
        Ok(Some(ctx.locale.tf("time_fetch_err", &[("location", &loc_display)])))
    }
}

fn translate_day(day: &str, lang: &str) -> &'static str {
    match lang {
        "nl" => match day.to_lowercase().as_str() {
            "monday" => "Maandag",
            "tuesday" => "Dinsdag",
            "wednesday" => "Woensdag",
            "thursday" => "Donderdag",
            "friday" => "Vrijdag",
            "saturday" => "Zaterdag",
            "sunday" => "Zondag",
            _ => "",
        },
        "de" => match day.to_lowercase().as_str() {
            "monday" => "Montag",
            "tuesday" => "Dienstag",
            "wednesday" => "Mittwoch",
            "thursday" => "Donnerstag",
            "friday" => "Freitag",
            "saturday" => "Samstag",
            "sunday" => "Sonntag",
            _ => "",
        },
        _ => match day.to_lowercase().as_str() {
            "monday" => "Monday",
            "tuesday" => "Tuesday",
            "wednesday" => "Wednesday",
            "thursday" => "Thursday",
            "friday" => "Friday",
            "saturday" => "Saturday",
            "sunday" => "Sunday",
            _ => "",
        },
    }
}

fn urlencoding_simple(s: &str) -> String {
    s.trim().replace(' ', "%20")
}
