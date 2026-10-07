//! Nederlandse schoolvakanties via de officiële open data API van Rijksoverheid
//! (https://opendata.rijksoverheid.nl/v1/infotypes/schoolholidays). Keyless en gratis.
//! De data wordt maximaal één keer per jaar opgehaald en lokaal gecachet in `data/`.

use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime};
use std::path::Path;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

const API_URL: &str = "https://opendata.rijksoverheid.nl/v1/infotypes/schoolholidays?output=json&rows=50";
const CACHE_FILE: &str = "data/schoolvakanties.json";
const MAX_CACHE_AGE: Duration = Duration::from_secs(365 * 24 * 3600);

const MONTHS: [&str; 12] = ["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"];

const REGION_INFO: [&str; 3] = [
    "🌍 Noord: Groningen, Friesland, Drenthe, Overijssel, Noord-Holland en Flevoland (m.u.v. Zeewolde)",
    "🌍 Midden: Zuid-Holland, Utrecht, Zeewolde en delen Gelderland",
    "🌍 Zuid: Zeeland, Noord-Brabant, Limburg en delen Gelderland",
];

#[derive(Debug, Clone)]
struct Holiday {
    name: String,
    /// `false` = adviesdata (zoals de meivakantie), geen verplichte data.
    compulsory: bool,
    /// (regio in kleine letters, start, eind inclusief)
    entries: Vec<(String, NaiveDate, NaiveDate)>,
}

/// Regio's met dezelfde periode samengevoegd, bijv. "Midden/Zuid 17-25 okt".
#[derive(Debug, Clone)]
struct Span {
    regions: Vec<String>,
    start: NaiveDate,
    end: NaiveDate,
}

impl Holiday {
    fn entries_for(&self, filter: Option<&str>) -> impl Iterator<Item = &(String, NaiveDate, NaiveDate)> {
        let filter = filter.map(str::to_string);
        self.entries
            .iter()
            .filter(move |(r, _, _)| match &filter {
                Some(f) => r == f || r == "heel nederland",
                None => true,
            })
    }

    fn spans(&self, filter: Option<&str>) -> Vec<Span> {
        let mut spans: Vec<Span> = Vec::new();
        for (region, start, end) in self.entries_for(filter) {
            match spans.iter_mut().find(|s| s.start == *start && s.end == *end) {
                Some(s) => s.regions.push(region.clone()),
                None => spans.push(Span { regions: vec![region.clone()], start: *start, end: *end }),
            }
        }
        spans.sort_by_key(|s| s.start);
        spans
    }

    fn first_start(&self, filter: Option<&str>) -> Option<NaiveDate> {
        self.entries_for(filter).map(|e| e.1).min()
    }

    fn last_end(&self, filter: Option<&str>) -> Option<NaiveDate> {
        self.entries_for(filter).map(|e| e.2).max()
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn region_label(regions: &[String]) -> String {
    let mut sorted: Vec<&String> = regions.iter().collect();
    let order = |r: &str| match r {
        "noord" => 0,
        "midden" => 1,
        "zuid" => 2,
        _ => 3,
    };
    sorted.sort_by_key(|r| order(r));
    sorted.iter().map(|r| capitalize(r)).collect::<Vec<_>>().join("/")
}

fn fmt_range(start: NaiveDate, end: NaiveDate) -> String {
    let (sm, em) = (MONTHS[start.month0() as usize], MONTHS[end.month0() as usize]);
    if start == end {
        format!("{} {}", start.day(), sm)
    } else if start.month() == end.month() && start.year() == end.year() {
        format!("{}-{} {}", start.day(), end.day(), sm)
    } else {
        format!("{} {}-{} {}", start.day(), sm, end.day(), em)
    }
}

fn fmt_spans(h: &Holiday, filter: Option<&str>) -> String {
    h.spans(filter)
        .iter()
        .map(|s| {
            let range = fmt_range(s.start, s.end);
            if s.regions.iter().all(|r| r == "heel nederland") {
                range
            } else {
                format!("{} {}", region_label(&s.regions), range)
            }
        })
        .collect::<Vec<_>>()
        .join(" • ")
}

fn emoji_for(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("herfst") {
        "🍂"
    } else if n.contains("kerst") {
        "🎄"
    } else if n.contains("voorjaar") {
        "🌸"
    } else if n.contains("mei") {
        "🌷"
    } else if n.contains("zomer") {
        "☀️"
    } else {
        "🏖️"
    }
}

/// De API levert UTC-tijdstippen: starts staan op 00:00Z (= de startdag) en eindes op 21:59Z of
/// 22:59Z (= 23:59 lokale tijd op de einddag). `+1u` maakt daar de juiste lokale einddag van.
fn api_start(s: &str) -> Option<NaiveDate> {
    NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f").ok().map(|d| d.date())
}

fn api_end(s: &str) -> Option<NaiveDate> {
    NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|d| (d + ChronoDuration::hours(1)).date())
}

fn parse_holidays(json: &str) -> Vec<Holiday> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in root.as_array().into_iter().flatten() {
        for content in item["content"].as_array().into_iter().flatten() {
            for vac in content["vacations"].as_array().into_iter().flatten() {
                let name = vac["type"].as_str().unwrap_or("").trim().to_string();
                if name.is_empty() {
                    continue;
                }
                let compulsory = vac["compulsorydates"].as_str().map(|v| v.trim() == "true").unwrap_or(true);
                let entries: Vec<_> = vac["regions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| {
                        let region = r["region"].as_str()?.trim().to_lowercase();
                        Some((region, api_start(r["startdate"].as_str()?)?, api_end(r["enddate"].as_str()?)?))
                    })
                    .collect();
                if !entries.is_empty() {
                    out.push(Holiday { name, compulsory, entries });
                }
            }
        }
    }
    out.sort_by_key(|h| h.first_start(None));
    out
}

/// Vakantie die nu bezig is of als eerstvolgende begint (regio-filter optioneel).
fn current_or_next<'a>(holidays: &'a [Holiday], today: NaiveDate, filter: Option<&str>) -> Option<&'a Holiday> {
    holidays
        .iter()
        .filter(|h| h.last_end(filter).map(|e| e >= today).unwrap_or(false))
        .min_by_key(|h| h.first_start(filter))
}

fn describe_current_or_next(holidays: &[Holiday], today: NaiveDate, filter: Option<&str>) -> String {
    let Some(h) = current_or_next(holidays, today, filter) else {
        return "📚 Geen schoolvakanties meer bekend in de officiële data.".to_string();
    };
    let spans = h.spans(filter);
    let active: Vec<&Span> = spans.iter().filter(|s| s.start <= today && today <= s.end).collect();
    let advies = if h.compulsory { "" } else { " (advies)" };
    let emoji = emoji_for(&h.name);

    if !active.is_empty() {
        let now = active
            .iter()
            .map(|s| format!("{} t/m {} (nog {} dagen)", region_label(&s.regions), fmt_range(s.end, s.end), (s.end - today).num_days()))
            .collect::<Vec<_>>()
            .join(" • ");
        format!("🏖️ Nu schoolvakantie: {} {}{}: {} | {}", emoji, h.name, advies, fmt_spans(h, filter), now)
    } else {
        let start = h.first_start(filter).unwrap_or(today);
        let days = (start - today).num_days();
        let when = if days == 1 { "morgen".to_string() } else { format!("over {} dagen", days) };
        format!(
            "📚 Momenteel geen schoolvakantie. Eerstvolgende: {} {}{} ({}): {}",
            emoji,
            h.name,
            advies,
            when,
            fmt_spans(h, filter)
        )
    }
}

fn describe_year(holidays: &[Holiday], year: i32, filter: Option<&str>) -> Vec<String> {
    let mut lines = vec![match filter {
        Some(f) => format!("📅 Schoolvakanties {} {}", capitalize(f), year),
        None => format!("📅 Schoolvakanties Nederland {}", year),
    }];
    let mut found = false;
    for h in holidays {
        let (Some(s), Some(e)) = (h.first_start(filter), h.last_end(filter)) else { continue };
        if s.year() != year && e.year() != year {
            continue;
        }
        // Kerstvakantie loopt over de jaargrens; overige vakanties horen bij één kalenderjaar.
        found = true;
        let kind = if h.compulsory { "" } else { " (advies)" };
        lines.push(format!("{} {}{}: {}", emoji_for(&h.name), h.name, kind, fmt_spans(h, filter)));
    }
    if !found {
        lines.push("Geen data bekend voor dit jaar.".to_string());
    } else if filter.is_none() {
        lines.extend(REGION_INFO.iter().map(|s| s.to_string()));
    }
    lines
}

fn parse_args(args: &str) -> (Option<i32>, Option<&'static str>) {
    let mut year = None;
    let mut region = None;
    for tok in args.split_whitespace() {
        let t = tok.to_lowercase();
        match t.as_str() {
            "noord" => region = Some("noord"),
            "midden" => region = Some("midden"),
            "zuid" => region = Some("zuid"),
            _ => {
                if let Ok(y) = t.parse::<i32>() {
                    if (2000..=2100).contains(&y) {
                        year = Some(y);
                    }
                }
            }
        }
    }
    (year, region)
}

pub struct VakantiePlugin {
    cache: Mutex<Option<Vec<Holiday>>>,
}

impl VakantiePlugin {
    pub fn new() -> Self {
        Self { cache: Mutex::new(None) }
    }

    fn cache_is_fresh(path: &Path) -> bool {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|age| age < MAX_CACHE_AGE)
            .unwrap_or(false)
    }

    /// Geeft de vakantiedata: uit geheugen, anders uit het cachebestand, anders (hooguit 1x per jaar) van de API.
    async fn holidays(&self, ctx: &PluginContext, today: NaiveDate) -> Option<Vec<Holiday>> {
        let mut guard = self.cache.lock().await;
        let usable = |h: &Vec<Holiday>| h.iter().any(|x| x.last_end(None).map(|e| e >= today).unwrap_or(false));

        if let Some(h) = guard.as_ref() {
            if usable(h) {
                return Some(h.clone());
            }
        }

        let path = Path::new(CACHE_FILE);
        let cached = std::fs::read_to_string(path).ok().map(|s| parse_holidays(&s)).filter(|h| !h.is_empty());
        if let Some(h) = &cached {
            if usable(h) && Self::cache_is_fresh(path) {
                *guard = cached.clone();
                return cached;
            }
        }

        // Verversen via de officiële Rijksoverheid API
        match ctx.http.get(API_URL).timeout(Duration::from_secs(10)).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    let parsed = parse_holidays(&body);
                    if !parsed.is_empty() {
                        if let Some(dir) = path.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        let _ = std::fs::write(path, &body);
                        *guard = Some(parsed.clone());
                        return Some(parsed);
                    }
                }
            }
            Ok(resp) => tracing::warn!("Schoolvakanties API gaf status {}", resp.status()),
            Err(e) => tracing::warn!("Schoolvakanties API niet bereikbaar: {}", e),
        }

        // Terugvallen op oude cache is beter dan niets
        if cached.is_some() {
            *guard = cached.clone();
        }
        cached
    }
}

#[async_trait]
impl Plugin for VakantiePlugin {
    fn name(&self) -> &'static str { "vakantie" }

    fn triggers(&self) -> &[&'static str] {
        &["vakantie", "vakanties", "schoolvakantie", "schoolvakanties"]
    }

    fn help(&self) -> &'static str {
        "!vakantie [noord|midden|zuid] - Huidige/volgende schoolvakantie | !vakanties [jaar] [regio] - Overzicht (bron: Rijksoverheid)"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let today = Local::now().date_naive();
        let Some(holidays) = self.holidays(ctx, today).await else {
            return Ok(Some("⚠️ Schoolvakanties konden niet worden opgehaald bij Rijksoverheid. Probeer het later opnieuw.".into()));
        };
        let (year, region) = parse_args(&cmd.args);

        if cmd.trigger.to_lowercase().ends_with("vakanties") {
            let lines = describe_year(&holidays, year.unwrap_or(today.year()), region);
            Ok(Some(lines.join(" | ")))
        } else {
            Ok(Some(describe_current_or_next(&holidays, today, region)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ingekorte kopie van de echte API-respons (schooljaar 2026-2027)
    const FIXTURE: &str = r#"[{"content":[{"schoolyear":"2026-2027","vacations":[
      {"type":"\n Herfstvakantie \n","compulsorydates":"false","regions":[
        {"region":"noord","startdate":"2026-10-10T00:00:00.000Z","enddate":"2026-10-18T21:59:00.000Z"},
        {"region":"midden","startdate":"2026-10-17T00:00:00.000Z","enddate":"2026-10-25T22:59:00.000Z"},
        {"region":"zuid","startdate":"2026-10-17T00:00:00.000Z","enddate":"2026-10-25T22:59:00.000Z"}]},
      {"type":"Kerstvakantie","compulsorydates":"true","regions":[
        {"region":"heel Nederland","startdate":"2026-12-19T00:00:00.000Z","enddate":"2027-01-03T22:59:00.000Z"}]},
      {"type":"Voorjaarsvakantie","compulsorydates":"false","regions":[
        {"region":"noord","startdate":"2027-02-20T00:00:00.000Z","enddate":"2027-02-28T22:59:00.000Z"},
        {"region":"midden","startdate":"2027-02-20T00:00:00.000Z","enddate":"2027-02-28T22:59:00.000Z"},
        {"region":"zuid","startdate":"2027-02-13T00:00:00.000Z","enddate":"2027-02-21T22:59:00.000Z"}]}
    ]}]}]"#;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn parses_and_converts_dates() {
        let h = parse_holidays(FIXTURE);
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].name, "Herfstvakantie");
        assert!(!h[0].compulsory);
        // 21:59Z en 22:59Z zijn beide de laatste dag (zondag) in lokale tijd
        assert_eq!(h[0].last_end(Some("noord")), Some(d(2026, 10, 18)));
        assert_eq!(h[0].last_end(Some("zuid")), Some(d(2026, 10, 25)));
        assert_eq!(h[1].last_end(None), Some(d(2027, 1, 3)));
    }

    #[test]
    fn groups_regions_with_same_dates() {
        let h = parse_holidays(FIXTURE);
        assert_eq!(fmt_spans(&h[0], None), "Noord 10-18 okt • Midden/Zuid 17-25 okt");
        assert_eq!(fmt_spans(&h[1], None), "19 dec-3 jan");
        assert_eq!(fmt_spans(&h[2], None), "Zuid 13-21 feb • Noord/Midden 20-28 feb");
        assert_eq!(fmt_spans(&h[0], Some("zuid")), "Zuid 17-25 okt");
    }

    #[test]
    fn next_and_current() {
        let h = parse_holidays(FIXTURE);
        let before = describe_current_or_next(&h, d(2026, 10, 7), None);
        assert!(before.contains("Herfstvakantie") && before.contains("over 3 dagen"), "{before}");
        let during = describe_current_or_next(&h, d(2026, 10, 12), None);
        assert!(during.contains("Nu schoolvakantie") && during.contains("Noord t/m 18 okt"), "{during}");
        let between = describe_current_or_next(&h, d(2026, 11, 20), None);
        assert!(between.contains("Kerstvakantie"), "{between}");
    }

    #[test]
    fn year_overview_and_args() {
        let h = parse_holidays(FIXTURE);
        let lines = describe_year(&h, 2026, None);
        assert_eq!(lines[0], "📅 Schoolvakanties Nederland 2026");
        assert!(lines.iter().any(|l| l.contains("Herfstvakantie")));
        assert!(lines.iter().any(|l| l.contains("Kerstvakantie")));
        assert!(!lines.iter().any(|l| l.contains("Voorjaarsvakantie")));
        assert_eq!(parse_args("2027 zuid"), (Some(2027), Some("zuid")));
        assert_eq!(parse_args(""), (None, None));
    }
}
