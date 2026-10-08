//! `!regen [plaats]`: neerslagverwachting voor de komende ~2 uur (Buienradar raintext, 5-minutenresolutie).
//! Bron: https://gpsgadget.buienradar.nl/data/raintext?lat=..&lon=.. (gratis weerdata van Buienradar, geen sleutel).
//! Locatie: plaatsnaam via de Open-Meteo geocoder (zelfde quotabewaking als !weer) of je vaste `!weer set`-plaats.

use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

pub struct RainPlugin;

/// (tijd "HH:MM", mm/uur)
type Sample = (String, f64);

/// Buienradar-waarde 0..255 naar mm/uur: `10^((v-109)/32)`; 0 = droog.
fn value_to_mmh(v: u32) -> f64 {
    if v == 0 { 0.0 } else { 10f64.powf((v as f64 - 109.0) / 32.0) }
}

fn parse_raintext(body: &str) -> Vec<Sample> {
    body.lines()
        .filter_map(|l| {
            let (v, t) = l.trim().split_once('|')?;
            let v: u32 = v.trim().parse().ok()?;
            let t = t.trim();
            let b = t.as_bytes();
            let is_time = b.len() == 5 && b[2] == b':' && [0, 1, 3, 4].iter().all(|&i| b[i].is_ascii_digit());
            is_time.then(|| (t.to_string(), value_to_mmh(v)))
        })
        .collect()
}

fn intensity(mmh: f64, dutch: bool) -> &'static str {
    match (mmh, dutch) {
        (m, true) if m < 0.5 => "lichte regen",
        (m, true) if m < 2.5 => "matige regen",
        (m, true) if m < 10.0 => "zware regen",
        (_, true) => "hevige regen",
        (m, false) if m < 0.5 => "light rain",
        (m, false) if m < 2.5 => "moderate rain",
        (m, false) if m < 10.0 => "heavy rain",
        (_, false) => "very heavy rain",
    }
}

fn bar(samples: &[Sample]) -> String {
    samples
        .iter()
        .map(|(_, m)| match *m {
            x if x < 0.1 => '▁',
            x if x < 0.5 => '▂',
            x if x < 1.0 => '▃',
            x if x < 2.5 => '▄',
            x if x < 5.0 => '▅',
            x if x < 10.0 => '▆',
            _ => '█',
        })
        .collect()
}

const RAIN_THRESHOLD: f64 = 0.1;

fn summarize(samples: &[Sample], dutch: bool) -> String {
    if samples.is_empty() {
        return if dutch { "geen gegevens ontvangen".into() } else { "no data received".into() };
    }
    let last = &samples[samples.len() - 1];
    let first_rain = samples.iter().position(|(_, m)| *m >= RAIN_THRESHOLD);
    let Some(i) = first_rain else {
        return if dutch {
            format!("☀️ Droog tot minstens {}", last.0)
        } else {
            format!("☀️ Dry until at least {}", last.0)
        };
    };
    // De eerste bui: van i tot de eerste droge meting erna (of het einde van het venster)
    let end = samples[i..].iter().position(|(_, m)| *m < RAIN_THRESHOLD).map(|j| i + j);
    let episode = &samples[i..end.unwrap_or(samples.len())];
    let peak = episode.iter().cloned().fold(("".to_string(), 0.0), |acc, s| if s.1 > acc.1 { s } else { acc });
    let first_dry_after = end.map(|e| &samples[e].0);
    // Komt er na de eerste bui nog een tweede?
    let second = end.and_then(|e| samples[e..].iter().find(|(_, m)| *m >= RAIN_THRESHOLD)).map(|(t, _)| t.clone());

    let what = intensity(peak.1, dutch);
    let peak_txt = format!("{} {:.1} mm/u {} {}", if dutch { "max." } else { "peak" }, peak.1, if dutch { "rond" } else { "around" }, peak.0);
    let mut out = match (i, first_dry_after, dutch) {
        (0, Some(dry), true) => format!("🌧️ Het regent nu ({}), droog vanaf {} ({})", what, dry, peak_txt),
        (0, None, true) => format!("🌧️ Het regent nu ({}) en blijft regenen tot minstens {} ({})", what, last.0, peak_txt),
        (0, Some(dry), false) => format!("🌧️ It is raining now ({}), dry from {} ({})", what, dry, peak_txt),
        (0, None, false) => format!("🌧️ It is raining now ({}) and keeps raining until at least {} ({})", what, last.0, peak_txt),
        (_, Some(dry), true) => format!("🌦️ Droog tot {}, dan {} tot {} ({})", samples[i].0, what, dry, peak_txt),
        (_, None, true) => format!("🌦️ Droog tot {}, daarna {} tot minstens {} ({})", samples[i].0, what, last.0, peak_txt),
        (_, Some(dry), false) => format!("🌦️ Dry until {}, then {} until {} ({})", samples[i].0, what, dry, peak_txt),
        (_, None, false) => format!("🌦️ Dry until {}, then {} until at least {} ({})", samples[i].0, what, last.0, peak_txt),
    };
    if let Some(t) = second {
        out.push_str(&if dutch { format!(", daarna weer regen vanaf {}", t) } else { format!(", then rain again from {}", t) });
    }
    out
}

#[derive(Deserialize)]
struct GeoResults {
    results: Option<Vec<GeoHit>>,
}
#[derive(Deserialize)]
struct GeoHit {
    name: String,
    latitude: f64,
    longitude: f64,
}

/// Buienradar dekt Nederland en omgeving.
fn in_coverage(lat: f64, lon: f64) -> bool {
    (49.0..=54.5).contains(&lat) && (2.0..=8.5).contains(&lon)
}

#[async_trait]
impl Plugin for RainPlugin {
    fn name(&self) -> &'static str { "rain" }
    fn triggers(&self) -> &[&'static str] { &["regen", "rain", "buien"] }
    fn help(&self) -> &'static str {
        "!regen [plaats] - regenverwachting voor de komende 2 uur (Buienradar; zonder plaats je vaste !weer set-plaats)"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let dutch = ctx.locale.is_dutch();
        let mut place = cmd.args.trim().to_string();
        if place.is_empty() {
            place = super::weather::get_user_location(ctx, &cmd.platform, &cmd.author).await?.unwrap_or_default();
        }
        if place.is_empty() {
            return Ok(Some(if dutch {
                "🌧️ Gebruik: !regen <plaats>, of stel je vaste plaats in met !weer set <plaats>".into()
            } else {
                "🌧️ Usage: !regen <place>, or set your default with !weather set <place>".into()
            }));
        }
        // "Utrecht, Netherlands" (opgeslagen als !weer set) -> alleen het eerste deel zoeken
        let query = place.split(',').next().unwrap_or(&place).trim().to_string();

        if let Err(block) = ctx.open_meteo_quota.check_and_increment_for_lang(ctx.locale.language()) {
            return Ok(Some(block));
        }
        let api_key = std::env::var("OPEN_METEO_API_KEY").ok().filter(|k| !k.trim().is_empty());
        let host = if api_key.is_some() { "customer-geocoding-api.open-meteo.com" } else { "geocoding-api.open-meteo.com" };
        let mut hit: Option<GeoHit> = None;
        // Eerst Nederland (voorkomt een andere "Utrecht"), anders wereldwijd
        for country in [Some("NL"), None] {
            let mut req = ctx.http.get(format!("https://{}/v1/search", host)).query(&[("name", query.as_str()), ("count", "1")]);
            if let Some(c) = country {
                req = req.query(&[("countryCode", c)]);
            }
            if let Some(k) = &api_key {
                req = req.query(&[("apikey", k.as_str())]);
            }
            if let Ok(resp) = req.header("User-Agent", "IRCordBot/1.0").timeout(Duration::from_secs(8)).send().await {
                if let Ok(geo) = resp.json::<GeoResults>().await {
                    if let Some(h) = geo.results.and_then(|r| r.into_iter().next()) {
                        hit = Some(h);
                        break;
                    }
                }
            }
        }
        let Some(hit) = hit else {
            return Ok(Some(format!("🌧️ {} '{}'.", if dutch { "Plaats niet gevonden:" } else { "Place not found:" }, query)));
        };
        if !in_coverage(hit.latitude, hit.longitude) {
            return Ok(Some(format!(
                "🌧️ {}: {}",
                hit.name,
                if dutch { "buiten het bereik van de Buienradar-regenvoorspelling (Nederland en omgeving)." } else { "outside Buienradar's rain-radar coverage (the Netherlands and surroundings)." }
            )));
        }

        let url = format!("https://gpsgadget.buienradar.nl/data/raintext?lat={:.2}&lon={:.2}", hit.latitude, hit.longitude);
        let body = match ctx.http.get(&url).header("User-Agent", "IRCordBot/1.0").timeout(Duration::from_secs(8)).send().await {
            Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
            _ => return Ok(Some(format!("⚠️ Buienradar: {}", if dutch { "de dienst is nu niet bereikbaar." } else { "the service is unreachable right now." }))),
        };
        let samples = parse_raintext(&body);
        if samples.is_empty() {
            return Ok(Some(format!("⚠️ Buienradar: {}", if dutch { "onverwacht antwoord." } else { "unexpected response." })));
        }
        Ok(Some(format!(
            "{} — \x02{}\x02 {}–{} {}",
            summarize(&samples, dutch),
            hit.name,
            samples.first().map(|s| s.0.as_str()).unwrap_or(""),
            samples.last().map(|s| s.0.as_str()).unwrap_or(""),
            bar(&samples)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Echt antwoord van Buienradar (Amsterdam), ingekort
    const LIVE: &str = "000|05:10\r\n087|05:15\r\n102|05:20\r\n087|05:25\r\n108|05:40\r\n000|06:10\r\n000|06:15\r\n087|06:50\r\n092|06:55\r\n";

    #[test]
    fn raintext_and_intensity_scale() {
        let s = parse_raintext(LIVE);
        assert_eq!(s.len(), 9);
        assert_eq!(s[0], ("05:10".to_string(), 0.0));
        // 087 => 10^((87-109)/32) = 0.206 mm/u; 108 => 0.93 mm/u; 255 => ~ 1.1e5? (formule schaalt, geen limiet nodig)
        assert!((s[1].1 - 0.2056).abs() < 0.001, "{}", s[1].1);
        assert!((value_to_mmh(109) - 1.0).abs() < 1e-9);
        assert!(parse_raintext("rommel\n12|ab:cd\n").is_empty());
    }

    #[test]
    fn summaries() {
        // droog
        let dry: Vec<Sample> = vec![("10:00".into(), 0.0), ("10:05".into(), 0.0)];
        assert_eq!(summarize(&dry, true), "☀️ Droog tot minstens 10:05");
        // regent nu en stopt
        let stops: Vec<Sample> = vec![("10:00".into(), 1.2), ("10:05".into(), 3.0), ("10:10".into(), 0.0)];
        let s = summarize(&stops, true);
        assert!(s.starts_with("🌧️ Het regent nu (zware regen), droog vanaf 10:10") && s.contains("3.0 mm/u rond 10:05"), "{s}");
        // droog, dan regen
        let later: Vec<Sample> = vec![("10:00".into(), 0.0), ("10:05".into(), 0.0), ("10:10".into(), 0.3), ("10:15".into(), 0.0)];
        let s = summarize(&later, true);
        assert!(s.starts_with("🌦️ Droog tot 10:10, dan lichte regen tot 10:15"), "{s}");
        // tweede bui: piek van de latere bui hoort niet bij de eerste
        let two: Vec<Sample> = vec![("10:00".into(), 0.0), ("10:05".into(), 0.3), ("10:10".into(), 0.0), ("10:15".into(), 8.0)];
        let s = summarize(&two, true);
        assert!(s.contains("lichte regen tot 10:10") && s.contains("max. 0.3 mm/u rond 10:05") && s.ends_with("daarna weer regen vanaf 10:15"), "{s}");
        // blijft regenen
        let on: Vec<Sample> = vec![("10:00".into(), 12.0), ("10:05".into(), 12.0)];
        assert!(summarize(&on, false).contains("keeps raining until at least 10:05"));
        assert!(summarize(&[], true).contains("geen gegevens"));
    }

    #[test]
    fn bar_and_coverage() {
        assert_eq!(bar(&[("a".into(), 0.0), ("b".into(), 0.3), ("c".into(), 1.5), ("d".into(), 20.0)]), "▁▂▄█");
        assert!(in_coverage(52.37, 4.89));
        assert!(!in_coverage(40.4, -3.7));
    }

    /// Echte aanroepen (netwerk nodig): `cargo test live_rain -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_rain_geocode_and_buienradar() {
        let http = reqwest::Client::new();
        let geo: GeoResults = http
            .get("https://geocoding-api.open-meteo.com/v1/search")
            .query(&[("name", "Utrecht"), ("count", "1"), ("countryCode", "NL")])
            .send().await.unwrap().json().await.unwrap();
        let hit = geo.results.unwrap().into_iter().next().unwrap();
        println!("GEO: {} {} {}", hit.name, hit.latitude, hit.longitude);
        assert!(in_coverage(hit.latitude, hit.longitude));
        let body = http
            .get(format!("https://gpsgadget.buienradar.nl/data/raintext?lat={:.2}&lon={:.2}", hit.latitude, hit.longitude))
            .send().await.unwrap().text().await.unwrap();
        let samples = parse_raintext(&body);
        println!("RAIN: {} samples; {} {}", samples.len(), summarize(&samples, true), bar(&samples));
        assert!(samples.len() >= 12);
    }
}
