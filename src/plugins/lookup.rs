//! Opzoek-commando's op vaste, gratis bronnen zonder API-sleutel:
//! `!postcode` (PDOK Locatieserver), `!ipinfo` (ipwho.is), `!domein` (RDAP), `!define` (Wiktionary),
//! `!qr` (api.qrserver.com), `!short` (TinyURL), `!xkcd`, `!joke` (JokeAPI) en `!grap` (lokale AI).
//!
//! Alle parseerfuncties zijn zuiver en getest met voorbeeldantwoorden die live zijn opgehaald.

use super::{CommandEvent, Plugin, PluginContext};
use crate::utils::ssrf;
use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

const UA: &str = "IRCordBot/1.0 (+https://github.com/Grandmasg/IRCord)";
const TIMEOUT: Duration = Duration::from_secs(8);

pub struct LookupPlugin;

fn tr(dutch: bool, nl: &str, en: &str) -> String {
    if dutch { nl.to_string() } else { en.to_string() }
}

// ============================ postcode (PDOK) ============================

#[derive(Debug, PartialEq)]
struct PostcodeQuery {
    postcode: String,
    number: Option<String>,
}

fn parse_postcode_args(args: &str) -> Option<PostcodeQuery> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^(\d{4})\s?([A-Za-z]{2})(?:\s*(\d{1,5}))?(?:\s*[A-Za-z0-9\-]{0,6})?$").unwrap());
    let c = re.captures(args.trim())?;
    Some(PostcodeQuery { postcode: format!("{}{}", &c[1], c[2].to_uppercase()), number: c.get(3).map(|m| m.as_str().to_string()) })
}

fn parse_pdok(json: &Value) -> Option<String> {
    let doc = json["response"]["docs"].as_array()?.first()?;
    let name = doc["weergavenaam"].as_str()?;
    let mut out = name.to_string();
    if let Some(g) = doc["gemeentenaam"].as_str() {
        out.push_str(&format!(" • gemeente {}", g));
    }
    if let Some(p) = doc["provincienaam"].as_str() {
        out.push_str(&format!(" • {}", p));
    }
    Some(out)
}

// ============================ ipinfo (ipwho.is) ============================

fn parse_ipwho(json: &Value) -> Result<String, String> {
    if json["success"].as_bool() != Some(true) {
        return Err(json["message"].as_str().unwrap_or("onbekende fout").to_string());
    }
    let s = |k: &str| json[k].as_str().unwrap_or("").to_string();
    let place: Vec<String> = [s("city"), s("region")].into_iter().filter(|x| !x.is_empty()).collect();
    let mut out = s("ip").to_string();
    let mut loc = place.join(", ");
    if !s("country_code").is_empty() {
        loc = format!("{} ({})", if loc.is_empty() { s("country") } else { loc }, s("country_code"));
    }
    if !loc.is_empty() {
        out.push_str(&format!(" • {}", loc));
    }
    let org = json["connection"]["org"].as_str().or(json["connection"]["isp"].as_str()).unwrap_or("");
    if !org.is_empty() {
        out.push_str(&format!(" • {}", org));
    }
    if let Some(asn) = json["connection"]["asn"].as_i64() {
        out.push_str(&format!(" • AS{}", asn));
    }
    if let Some(tz) = json["timezone"]["id"].as_str() {
        out.push_str(&format!(" • {}", tz));
    }
    Ok(out)
}

// ============================ domein (RDAP) ============================

fn valid_domain(d: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^([a-z0-9]([a-z0-9\-]{0,61}[a-z0-9])?\.)+[a-z]{2,24}$").unwrap());
    d.len() <= 253 && re.is_match(d)
}

fn event_date(json: &Value, action: &str) -> Option<NaiveDate> {
    json["events"]
        .as_array()?
        .iter()
        .find(|e| e["eventAction"].as_str() == Some(action))
        .and_then(|e| e["eventDate"].as_str())
        .and_then(|d| NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok())
}

fn registrar_name(json: &Value) -> Option<String> {
    fn find(entities: &Value) -> Option<String> {
        for e in entities.as_array()? {
            let is_registrar = e["roles"].as_array().map(|r| r.iter().any(|x| x.as_str() == Some("registrar"))).unwrap_or(false);
            if is_registrar {
                if let Some(items) = e["vcardArray"][1].as_array() {
                    for it in items {
                        if it[0].as_str() == Some("fn") {
                            return it[3].as_str().map(str::to_string);
                        }
                    }
                }
            }
            if let Some(n) = find(&e["entities"]) {
                return Some(n);
            }
        }
        None
    }
    find(&json["entities"])
}

fn describe_rdap(json: &Value, today: NaiveDate) -> String {
    let name = json["ldhName"].as_str().unwrap_or("?").to_lowercase();
    let fmt = |d: NaiveDate| d.format("%d-%m-%Y").to_string();
    let mut parts = vec![format!("🌍 {}", name)];
    if let Some(d) = event_date(json, "registration") {
        parts.push(format!("geregistreerd {}", fmt(d)));
    }
    if let Some(d) = event_date(json, "expiration") {
        let days = (d - today).num_days();
        let when = if days >= 0 { format!("nog {} dagen", days) } else { format!("{} dagen geleden verlopen", -days) };
        parts.push(format!("verloopt {} ({})", fmt(d), when));
    }
    if let Some(r) = registrar_name(json) {
        parts.push(format!("registrar: {}", r));
    }
    let ns: Vec<String> = json["nameservers"]
        .as_array()
        .map(|a| a.iter().filter_map(|n| n["ldhName"].as_str().map(|s| s.to_lowercase())).take(3).collect())
        .unwrap_or_default();
    if !ns.is_empty() {
        parts.push(format!("NS: {}", ns.join(", ")));
    }
    parts.join(" • ")
}

// ============================ define (Wiktionary) ============================

fn strip_html(s: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static STYLE: OnceLock<Regex> = OnceLock::new();
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    let style = STYLE.get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap());
    let script = SCRIPT.get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap());
    let tag = TAG.get_or_init(|| Regex::new(r"<[^>]*>").unwrap());
    let without_blocks = script.replace_all(&style.replace_all(s, ""), "").into_owned();
    let no_tags = tag.replace_all(&without_blocks, "");
    no_tags
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// en.wiktionary REST: {"en":[{"partOfSpeech":"Noun","definitions":[{"definition":"<html>"}]}]}
fn parse_en_wiktionary(json: &Value, max_defs: usize) -> Vec<String> {
    let mut out = Vec::new();
    let Some(entries) = json["en"].as_array() else { return out };
    for entry in entries.iter().take(2) {
        let pos = entry["partOfSpeech"].as_str().unwrap_or("").to_lowercase();
        for d in entry["definitions"].as_array().into_iter().flatten() {
            let raw = d["definition"].as_str().unwrap_or("");
            // geneste lijsten met voorbeelden horen er niet bij
            let head = raw.split("<ol").next().unwrap_or(raw).split("<ul").next().unwrap_or(raw);
            let text = strip_html(head);
            if !text.is_empty() && out.len() < max_defs {
                out.push(format!("({}) {}", pos, text));
            }
        }
    }
    out
}

fn clean_wikitext(s: &str) -> String {
    static LABEL: OnceLock<Regex> = OnceLock::new();
    static TEMPLATE: OnceLock<Regex> = OnceLock::new();
    static LINK: OnceLock<Regex> = OnceLock::new();
    static QUOTES: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    let label = LABEL.get_or_init(|| Regex::new(r"\{\{([A-Za-z\-]+)\|nld\}\}").unwrap());
    let template = TEMPLATE.get_or_init(|| Regex::new(r"\{\{[^{}]*\}\}").unwrap());
    let link = LINK.get_or_init(|| Regex::new(r"\[\[(?:[^\]|]*\|)?([^\]]*)\]\]").unwrap());
    let quotes = QUOTES.get_or_init(|| Regex::new(r"'{2,}").unwrap());
    let tag = TAG.get_or_init(|| Regex::new(r"<[^>]*>").unwrap());
    let mut t = label.replace_all(s, "($1)").into_owned();
    for _ in 0..4 {
        let next = template.replace_all(&t, "").into_owned();
        if next == t {
            break;
        }
        t = next;
    }
    let t = link.replace_all(&t, "$1");
    let t = quotes.replace_all(&t, "");
    let t = tag.replace_all(&t, "");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// nl.wiktionary wikitext: de Nederlandse sectie begint met `{{=nld=}}`; definities zijn regels met één `#`.
fn nl_definitions(wikitext: &str, max_defs: usize) -> Vec<String> {
    let Some(section) = wikitext.split("{{=nld=}}").nth(1) else { return Vec::new() };
    let section = section.split("\n{{=").next().unwrap_or(section);
    let mut out = Vec::new();
    for line in section.lines() {
        if line.starts_with('#') && !line.starts_with("##") && !line.starts_with("#:") && !line.starts_with("#*") {
            let d = clean_wikitext(&line[1..]);
            if !d.is_empty() && out.len() < max_defs {
                out.push(d);
            }
        }
    }
    out
}

fn valid_term(t: &str) -> bool {
    let n = t.chars().count();
    (1..=60).contains(&n) && t.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\''))
}

// ============================ qr / xkcd / joke ============================

fn qr_url(text: &str) -> Option<String> {
    reqwest::Url::parse_with_params("https://api.qrserver.com/v1/create-qr-code/", [("size", "300x300"), ("margin", "10"), ("data", text)])
        .ok()
        .map(|u| u.to_string())
}

fn parse_xkcd(json: &Value) -> Option<String> {
    let num = json["num"].as_u64()?;
    let title = json["safe_title"].as_str().or(json["title"].as_str())?;
    let alt: String = json["alt"].as_str().unwrap_or("").chars().take(150).collect();
    Some(format!("🖼️ xkcd #{}: {} — {} https://xkcd.com/{}/", num, title, alt, num))
}

fn parse_joke(json: &Value) -> Option<String> {
    if json["error"].as_bool() == Some(true) {
        return None;
    }
    match json["type"].as_str()? {
        "single" => json["joke"].as_str().map(|j| j.split_whitespace().collect::<Vec<_>>().join(" ")),
        "twopart" => Some(format!("{} … {}", json["setup"].as_str()?.trim(), json["delivery"].as_str()?.trim())),
        _ => None,
    }
}

// ============================ plugin ============================

impl LookupPlugin {
    async fn get_json(ctx: &PluginContext, url: reqwest::Url) -> Result<(u16, Value), reqwest::Error> {
        let resp = ctx.http.get(url).header("User-Agent", UA).header("Accept", "application/json").timeout(TIMEOUT).send().await?;
        let status = resp.status().as_u16();
        let body = resp.json::<Value>().await.unwrap_or(Value::Null);
        Ok((status, body))
    }

    fn url_with_segment(base: &str, segment: &str) -> Option<reqwest::Url> {
        let mut u = reqwest::Url::parse(base).ok()?;
        u.path_segments_mut().ok()?.pop_if_empty().push(segment);
        Some(u)
    }
}

#[async_trait]
impl Plugin for LookupPlugin {
    fn name(&self) -> &'static str { "lookup" }
    fn triggers(&self) -> &[&'static str] {
        &["postcode", "ipinfo", "domein", "domain", "define", "qr", "short", "xkcd", "joke", "grap"]
    }
    fn help(&self) -> &'static str {
        "!postcode 1012JS [nr] | !ipinfo <ip> | !domein <naam> | !define [nl|en] <woord> | !qr <tekst> | !short <url> | !xkcd [nr|random] | !joke | !grap"
    }

    async fn on_command(&self, ctx: &PluginContext, cmd: &CommandEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let dutch = ctx.locale.is_dutch();
        let args = cmd.args.trim();
        let net_err = |what: &str| Ok(Some(format!("⚠️ {}: {}", what, tr(dutch, "de dienst is nu niet bereikbaar.", "the service is unreachable right now."))));

        match cmd.trigger.as_str() {
            "postcode" => {
                let Some(q) = parse_postcode_args(args) else {
                    return Ok(Some(tr(dutch, "📍 Gebruik: !postcode 1012JS of !postcode 1012JS 1 (met huisnummer)", "📍 Usage: !postcode 1012JS or !postcode 1012JS 1 (with house number)")));
                };
                let mut url = reqwest::Url::parse("https://api.pdok.nl/bzk/locatieserver/search/v3_1/free").unwrap();
                let (qstr, fq) = match &q.number {
                    Some(n) => (format!("postcode:{} AND huisnummer:{}", q.postcode, n), "type:adres"),
                    None => (format!("postcode:{}", q.postcode), "type:postcode"),
                };
                url.query_pairs_mut()
                    .append_pair("q", &qstr)
                    .append_pair("fq", fq)
                    .append_pair("rows", "1")
                    .append_pair("fl", "weergavenaam,gemeentenaam,provincienaam");
                match Self::get_json(ctx, url).await {
                    Ok((200, json)) => Ok(Some(match parse_pdok(&json) {
                        Some(p) => format!("📍 {}", p),
                        None => format!("📍 {} {}", tr(dutch, "Niets gevonden voor", "Nothing found for"), q.postcode),
                    })),
                    _ => net_err("PDOK"),
                }
            }
            "ipinfo" => {
                let Ok(ip) = args.parse::<std::net::IpAddr>() else {
                    return Ok(Some(tr(dutch, "🌐 Gebruik: !ipinfo <IPv4- of IPv6-adres>", "🌐 Usage: !ipinfo <IPv4 or IPv6 address>")));
                };
                if !ssrf::is_public_ip(ip) {
                    return Ok(Some(tr(dutch, "🌐 Dit is een privé- of intern adres; daar valt niets over op te zoeken.", "🌐 That is a private or internal address; there is nothing to look up.")));
                }
                let Some(url) = Self::url_with_segment("https://ipwho.is/", &ip.to_string()) else { return net_err("ipwho.is") };
                match Self::get_json(ctx, url).await {
                    Ok((_, json)) => Ok(Some(match parse_ipwho(&json) {
                        Ok(s) => format!("🌐 {}", s),
                        Err(e) => format!("🌐 ⚠️ {}", e),
                    })),
                    Err(_) => net_err("ipwho.is"),
                }
            }
            "domein" | "domain" => {
                let domain = args.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_lowercase();
                if !valid_domain(&domain) {
                    return Ok(Some(tr(dutch, "🌍 Gebruik: !domein tweakers.net", "🌍 Usage: !domein example.com")));
                }
                let Some(url) = Self::url_with_segment("https://rdap.org/domain/", &domain) else { return net_err("RDAP") };
                match Self::get_json(ctx, url).await {
                    Ok((200, json)) => Ok(Some(describe_rdap(&json, Local::now().date_naive()))),
                    Ok((404, _)) => Ok(Some(format!("🌍 {}: {}", domain, tr(dutch, "niet gevonden (niet geregistreerd, of deze extensie heeft geen RDAP)", "not found (unregistered, or this TLD has no RDAP)")))),
                    _ => net_err("RDAP"),
                }
            }
            "define" => {
                let mut words = args.splitn(2, char::is_whitespace);
                let first = words.next().unwrap_or("");
                let (lang, term) = match first.to_lowercase().as_str() {
                    "nl" => ("nl", words.next().unwrap_or("").trim().to_string()),
                    "en" => ("en", words.next().unwrap_or("").trim().to_string()),
                    _ => (if dutch { "nl" } else { "en" }, args.to_string()),
                };
                if !valid_term(&term) {
                    return Ok(Some(tr(dutch, "📖 Gebruik: !define [nl|en] <woord>", "📖 Usage: !define [nl|en] <word>")));
                }
                let defs = if lang == "en" {
                    let Some(url) = Self::url_with_segment("https://en.wiktionary.org/api/rest_v1/page/definition/", &term.to_lowercase()) else { return net_err("Wiktionary") };
                    match Self::get_json(ctx, url).await {
                        Ok((200, json)) => parse_en_wiktionary(&json, 3),
                        Ok(_) => Vec::new(),
                        Err(_) => return net_err("Wiktionary"),
                    }
                } else {
                    let mut url = reqwest::Url::parse("https://nl.wiktionary.org/w/api.php").unwrap();
                    url.query_pairs_mut()
                        .append_pair("action", "parse")
                        .append_pair("page", &term)
                        .append_pair("prop", "wikitext")
                        .append_pair("format", "json")
                        .append_pair("formatversion", "2")
                        .append_pair("redirects", "1");
                    match Self::get_json(ctx, url).await {
                        Ok((200, json)) => json["parse"]["wikitext"].as_str().map(|w| nl_definitions(w, 3)).unwrap_or_default(),
                        Ok(_) => Vec::new(),
                        Err(_) => return net_err("Wiktionary"),
                    }
                };
                if defs.is_empty() {
                    return Ok(Some(format!("📖 {} '{}' ({})", tr(dutch, "Geen definitie gevonden voor", "No definition found for"), term, lang)));
                }
                let numbered: Vec<String> = defs.iter().enumerate().map(|(i, d)| format!("{}) {}", i + 1, d)).collect();
                Ok(Some(format!("📖 \x02{}\x02 [{}, Wiktionary]: {}", term, lang, numbered.join(" ")))
                )
            }
            "qr" => {
                if args.is_empty() || args.chars().count() > 300 {
                    return Ok(Some(tr(dutch, "📱 Gebruik: !qr <tekst of url> (max. 300 tekens)", "📱 Usage: !qr <text or url> (max. 300 characters)")));
                }
                Ok(qr_url(args).map(|u| format!("📱 QR: {}", u)).or_else(|| Some("📱 ⚠️ QR kon niet worden gemaakt.".into())))
            }
            "short" => {
                if !ssrf::is_safe_public_url(args) {
                    return Ok(Some(tr(dutch, "🔗 Gebruik: !short <http(s)-url> (alleen publieke adressen)", "🔗 Usage: !short <http(s) url> (public addresses only)")));
                }
                let mut url = reqwest::Url::parse("https://tinyurl.com/api-create.php").unwrap();
                url.query_pairs_mut().append_pair("url", args);
                match ctx.http.get(url).header("User-Agent", UA).timeout(TIMEOUT).send().await {
                    Ok(r) if r.status().is_success() => {
                        let body = r.text().await.unwrap_or_default();
                        let short = body.trim();
                        if short.starts_with("https://tinyurl.com/") {
                            Ok(Some(format!("🔗 {}", short)))
                        } else {
                            Ok(Some("🔗 ⚠️ TinyURL gaf geen geldige korte link terug.".into()))
                        }
                    }
                    _ => net_err("TinyURL"),
                }
            }
            "xkcd" => {
                let latest = match Self::get_json(ctx, reqwest::Url::parse("https://xkcd.com/info.0.json").unwrap()).await {
                    Ok((200, j)) => j["num"].as_u64().unwrap_or(0),
                    _ => return net_err("xkcd"),
                };
                let wanted = match args.to_lowercase().as_str() {
                    "" => latest,
                    "random" | "willekeurig" => 1 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as u64 % latest.max(1)),
                    n => match n.trim_start_matches('#').parse::<u64>() {
                        Ok(n) if (1..=latest).contains(&n) => n,
                        _ => return Ok(Some(format!("🖼️ {}", tr(dutch, &format!("Kies een nummer van 1 t/m {}, of 'random'.", latest), &format!("Pick a number from 1 to {}, or 'random'.", latest))))),
                    },
                };
                let Some(url) = Self::url_with_segment(&format!("https://xkcd.com/{}/", wanted), "info.0.json") else { return net_err("xkcd") };
                match Self::get_json(ctx, url).await {
                    Ok((200, j)) => Ok(Some(parse_xkcd(&j).unwrap_or_else(|| "🖼️ ⚠️ Onverwacht antwoord van xkcd.".into()))),
                    Ok(_) => Ok(Some(format!("🖼️ xkcd #{}: {}", wanted, tr(dutch, "bestaat niet (nummer 404 is een bekende grap).", "does not exist (number 404 is a known joke)."))),
                    ),
                    _ => net_err("xkcd"),
                }
            }
            "joke" => {
                let mut url = reqwest::Url::parse("https://v2.jokeapi.dev/joke/Any").unwrap();
                url.query_pairs_mut().append_pair("safe-mode", "").append_pair("blacklistFlags", "nsfw,religious,political,racist,sexist,explicit");
                match Self::get_json(ctx, url).await {
                    Ok((200, j)) => Ok(Some(match parse_joke(&j) {
                        Some(t) => format!("😄 {}", t),
                        None => "😄 ⚠️ Geen grap ontvangen.".into(),
                    })),
                    _ => net_err("JokeAPI"),
                }
            }
            "grap" => {
                if !ctx.ai_manager.can_consume(120) {
                    return Ok(Some(format!("⚠️ {}", ctx.locale.t("ai_budget_exceeded"))));
                }
                let model = ctx.ai_manager.get_model();
                let system = if dutch {
                    "Je bent een grappenmaker in een gezellig Nederlands chatkanaal. Vertel precies één korte, originele, nette Nederlandse grap van maximaal twee zinnen. Geen discriminatie, seks of geweld. Geef alleen de grap."
                } else {
                    "You are a joke teller in a friendly chat channel. Tell exactly one short, original, clean joke of at most two sentences. No discrimination, sex or violence. Output only the joke."
                };
                let topic = if args.is_empty() { "iets willekeurigs".to_string() } else { args.chars().take(60).collect() };
                let prompt = format!("Onderwerp: {}", topic);
                match ctx.ai_client.ask_with_system(system, &cmd.author, &prompt, Some(&model)).await {
                    Ok(j) => {
                        ctx.ai_manager.record_consumption(90);
                        Ok(Some(format!("😄 {}", j.trim().trim_matches('"').split_whitespace().collect::<Vec<_>>().join(" "))))
                    }
                    Err(_) => net_err("AI"),
                }
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn postcode_parsing() {
        assert_eq!(parse_postcode_args("1012js"), Some(PostcodeQuery { postcode: "1012JS".into(), number: None }));
        assert_eq!(parse_postcode_args("1012 JS 1"), Some(PostcodeQuery { postcode: "1012JS".into(), number: Some("1".into()) }));
        assert_eq!(parse_postcode_args("1012JS 12a"), Some(PostcodeQuery { postcode: "1012JS".into(), number: Some("12".into()) }));
        assert_eq!(parse_postcode_args("amsterdam"), None);
        assert_eq!(parse_postcode_args("1012JS 1 OR 1=1; DROP"), None);
    }

    #[test]
    fn pdok_response_from_live_sample() {
        let j = json!({"response":{"numFound":1,"docs":[{"woonplaatsnaam":"Amsterdam","weergavenaam":"Dam 1, 1012JS Amsterdam","gemeentenaam":"Amsterdam","postcode":"1012JS","provincienaam":"Noord-Holland"}]}});
        assert_eq!(parse_pdok(&j).unwrap(), "Dam 1, 1012JS Amsterdam • gemeente Amsterdam • Noord-Holland");
        assert!(parse_pdok(&json!({"response":{"docs":[]}})).is_none());
    }

    #[test]
    fn ipwho_response() {
        let ok = json!({"ip":"8.8.8.8","success":true,"country":"United States","country_code":"US","region":"California","city":"San Jose",
            "connection":{"asn":15169,"org":"Google LLC","isp":"Google LLC"},"timezone":{"id":"America/Los_Angeles"}});
        assert_eq!(parse_ipwho(&ok).unwrap(), "8.8.8.8 • San Jose, California (US) • Google LLC • AS15169 • America/Los_Angeles");
        assert_eq!(parse_ipwho(&json!({"success":false,"message":"Invalid IP address"})).unwrap_err(), "Invalid IP address");
    }

    #[test]
    fn domain_validation_and_rdap_summary() {
        assert!(valid_domain("tweakers.net") && valid_domain("sub.example.co.uk"));
        for bad in ["localhost", "a_b.com", "-x.com", "x.c", "exa mple.com", "../etc", "a.com/../x"] {
            assert!(!valid_domain(bad), "{bad}");
        }
        let j = json!({"ldhName":"TWEAKERS.NET","events":[
            {"eventAction":"registration","eventDate":"1999-02-04T05:00:00Z"},
            {"eventAction":"expiration","eventDate":"2027-08-05T12:11:20Z"}],
            "entities":[{"roles":["registrar"],"vcardArray":["vcard",[["version",{},"text","4.0"],["fn",{},"text","Key-Systems GmbH"]]]}],
            "nameservers":[{"ldhName":"NS1.EXAMPLE.NET"},{"ldhName":"ns2.example.net"}]});
        let s = describe_rdap(&j, NaiveDate::from_ymd_opt(2026, 10, 8).unwrap());
        assert_eq!(s, "🌍 tweakers.net • geregistreerd 04-02-1999 • verloopt 05-08-2027 (nog 301 dagen) • registrar: Key-Systems GmbH • NS: ns1.example.net, ns2.example.net");
        let expired = describe_rdap(&json!({"ldhName":"x.nl","events":[{"eventAction":"expiration","eventDate":"2026-10-01"}]}), NaiveDate::from_ymd_opt(2026, 10, 8).unwrap());
        assert!(expired.contains("7 dagen geleden verlopen"), "{expired}");
    }

    #[test]
    fn english_definitions_strip_html_and_examples() {
        let j = json!({"en":[{"partOfSpeech":"Noun","definitions":[
            {"definition":""},
            {"definition":"A <a href=\"/wiki/challenge\">challenge</a>, <b>trial</b>.\n\n<ol><li>An exam</li></ol>"},
            {"definition":"Something &amp; more"},
            {"definition":"A home. <style data-mw=\"x\">.mw-parser-output .defdate{font-size:smaller}</style>"}]}]});
        assert_eq!(parse_en_wiktionary(&j, 3), vec!["(noun) A challenge, trial.", "(noun) Something & more", "(noun) A home."]);
    }

    #[test]
    fn dutch_wikitext_definitions() {
        let wt = "{{=nld=}}\n{{-pron-}}\n*x\n{{-noun-|nld}}\n#{{bouwkunde|nld}}, {{wonen|nld}} gebouw bestemd om in te [[wonen]]\n{{bijv-1|Zij wonen in een ''{{pn}}''.}}\n#geheel van [[afkomst|nakomelingen]]\n#:voorbeeld\n##sub\n#iets met ''cursief''\n{{-syn-}}\n*[1] [[woonhuis]]\n{{=eng=}}\n#english def";
        let d = nl_definitions(wt, 5);
        assert_eq!(d, vec!["(bouwkunde), (wonen) gebouw bestemd om in te wonen", "geheel van nakomelingen", "iets met cursief"]);
        assert!(nl_definitions("{{=eng=}}\n#x", 3).is_empty());
        assert!(valid_term("huis") && valid_term("op-en-top") && !valid_term("a<script>") && !valid_term(""));
    }

    #[test]
    fn qr_xkcd_and_joke() {
        let u = qr_url("hallo wereld & meer").unwrap();
        assert!(u.starts_with("https://api.qrserver.com/v1/create-qr-code/?size=300x300") && u.contains("data=hallo+wereld+%26+meer"), "{u}");
        let x = json!({"num":614,"safe_title":"Woodpecker","alt":"If you don't have an extension cord I can get that too."});
        assert_eq!(parse_xkcd(&x).unwrap(), "🖼️ xkcd #614: Woodpecker — If you don't have an extension cord I can get that too. https://xkcd.com/614/");
        assert_eq!(parse_joke(&json!({"error":false,"type":"single","joke":"Debugging:  Removing\nthe needles."})).unwrap(), "Debugging: Removing the needles.");
        assert_eq!(parse_joke(&json!({"error":false,"type":"twopart","setup":"Why?","delivery":"Because."})).unwrap(), "Why? … Because.");
        assert!(parse_joke(&json!({"error":true})).is_none());
    }

    /// Echte aanroepen naar de externe diensten (netwerk nodig): `cargo test live_ -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_services_parse_with_real_responses() {
        let http = reqwest::Client::new();
        let get = |url: reqwest::Url| {
            let http = http.clone();
            async move { http.get(url).header("User-Agent", UA).send().await.unwrap().json::<Value>().await.unwrap() }
        };

        let mut u = reqwest::Url::parse("https://api.pdok.nl/bzk/locatieserver/search/v3_1/free").unwrap();
        u.query_pairs_mut().append_pair("q", "postcode:1012JS AND huisnummer:1").append_pair("fq", "type:adres").append_pair("rows", "1")
            .append_pair("fl", "weergavenaam,gemeentenaam,provincienaam");
        let pdok = parse_pdok(&get(u).await);
        println!("PDOK: {:?}", pdok);
        assert!(pdok.unwrap().contains("Amsterdam"));

        let ip = parse_ipwho(&get(LookupPlugin::url_with_segment("https://ipwho.is/", "8.8.8.8").unwrap()).await);
        println!("IPWHO: {:?}", ip);
        assert!(ip.unwrap().contains("Google"));

        let rdap = describe_rdap(&get(LookupPlugin::url_with_segment("https://rdap.org/domain/", "tweakers.net").unwrap()).await, Local::now().date_naive());
        println!("RDAP: {}", rdap);
        assert!(rdap.contains("tweakers.net") && rdap.contains("verloopt"));

        let en = parse_en_wiktionary(&get(LookupPlugin::url_with_segment("https://en.wiktionary.org/api/rest_v1/page/definition/", "house").unwrap()).await, 3);
        println!("EN: {:?}", en);
        assert!(!en.is_empty());

        let mut u = reqwest::Url::parse("https://nl.wiktionary.org/w/api.php").unwrap();
        u.query_pairs_mut().append_pair("action", "parse").append_pair("page", "huis").append_pair("prop", "wikitext")
            .append_pair("format", "json").append_pair("formatversion", "2").append_pair("redirects", "1");
        let nl = nl_definitions(get(u).await["parse"]["wikitext"].as_str().unwrap(), 3);
        println!("NL: {:?}", nl);
        assert!(!nl.is_empty());

        let x = parse_xkcd(&get(reqwest::Url::parse("https://xkcd.com/614/info.0.json").unwrap()).await);
        println!("XKCD: {:?}", x);
        assert!(x.unwrap().contains("Woodpecker"));

        let mut u = reqwest::Url::parse("https://v2.jokeapi.dev/joke/Any").unwrap();
        u.query_pairs_mut().append_pair("safe-mode", "").append_pair("blacklistFlags", "nsfw,religious,political,racist,sexist,explicit");
        let joke = parse_joke(&get(u).await);
        println!("JOKE: {:?}", joke);
        assert!(joke.is_some());

        let mut u = reqwest::Url::parse("https://tinyurl.com/api-create.php").unwrap();
        u.query_pairs_mut().append_pair("url", "https://github.com/Grandmasg/IRCord");
        let short = http.get(u).send().await.unwrap().text().await.unwrap();
        println!("TINYURL: {}", short);
        assert!(short.starts_with("https://tinyurl.com/"));
    }
}
