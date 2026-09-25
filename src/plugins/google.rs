use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use scraper::{Html, Selector};
use serde::Deserialize;
use std::sync::OnceLock;

static RESULT_SEL: OnceLock<Selector> = OnceLock::new();
static TITLE_SEL: OnceLock<Selector> = OnceLock::new();
static SNIPPET_SEL: OnceLock<Selector> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct DuckDuckGoInstant {
    #[serde(rename = "Heading")]
    pub heading: Option<String>,
    #[serde(rename = "AbstractText")]
    pub abstract_text: Option<String>,
    #[serde(rename = "AbstractURL")]
    pub abstract_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleCseResponse {
    pub items: Option<Vec<GoogleCseItem>>,
}

#[derive(Debug, Deserialize)]
struct GoogleCseItem {
    pub title: Option<String>,
    pub link: Option<String>,
    pub snippet: Option<String>,
}

fn encode_query(input: &str) -> String {
    let mut encoded = String::new();
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{:02X}", b)),
        }
    }
    encoded
}

pub struct GooglePlugin;

impl GooglePlugin {
    async fn search_google_cse(
        http: &reqwest::Client,
        api_key: &str,
        cx: &str,
        query: &str,
    ) -> Option<String> {
        let url = format!(
            "https://www.googleapis.com/customsearch/v1?key={}&cx={}&q={}&num=1",
            api_key,
            cx,
            encode_query(query)
        );

        let resp = http
            .get(&url)
            .timeout(std::time::Duration::from_secs(6))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let data = resp.json::<GoogleCseResponse>().await.ok()?;
        let item = data.items?.into_iter().next()?;

        let title = item.title.unwrap_or_default();
        let link = item.link.unwrap_or_default();
        let snippet = item.snippet.unwrap_or_default().replace('\n', " ");

        Some(format!(
            "🔍 [Google] \x02{}\x02 - {} | {}",
            title, snippet, link
        ))
    }

    async fn search_ddg_html(http: &reqwest::Client, query: &str) -> Option<String> {
        let encoded = encode_query(query);
        let url = format!("https://html.duckduckgo.com/html/?q={}", encoded);

        let resp = http
            .post(&url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .header("Accept", "text/html,application/xhtml+xml")
            .timeout(std::time::Duration::from_secs(6))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let body = resp.text().await.ok()?;
        let document = Html::parse_document(&body);

        let result_sel = RESULT_SEL.get_or_init(|| Selector::parse(".result").unwrap());
        let title_sel = TITLE_SEL.get_or_init(|| Selector::parse(".result__a").unwrap());
        let snippet_sel = SNIPPET_SEL.get_or_init(|| Selector::parse(".result__snippet").unwrap());

        for element in document.select(result_sel) {
            if let Some(title_el) = element.select(title_sel).next() {
                let title = title_el.text().collect::<Vec<_>>().join(" ").trim().to_string();
                let raw_link = title_el.value().attr("href").unwrap_or_default();

                // Negeer DuckDuckGo advertentielinks
                if title.is_empty() || raw_link.is_empty() || raw_link.contains("duckduckgo.com/y.js") {
                    continue;
                }

                // Ontrafel uddg redirect
                let clean_link = if raw_link.contains("uddg=") {
                    raw_link
                        .split("uddg=")
                        .nth(1)
                        .and_then(|u| u.split('&').next())
                        .map(|encoded_u| {
                            percent_decode(encoded_u)
                        })
                        .unwrap_or_else(|| raw_link.to_string())
                } else if raw_link.starts_with("//") {
                    format!("https:{}", raw_link)
                } else {
                    raw_link.to_string()
                };

                let snippet = element
                    .select(snippet_sel)
                    .next()
                    .map(|s| s.text().collect::<Vec<_>>().join(" ").trim().to_string())
                    .unwrap_or_default();

                if !snippet.is_empty() {
                    return Some(format!(
                        "🔍 [Web Result] \x02{}\x02 - {} | {}",
                        title, snippet, clean_link
                    ));
                } else {
                    return Some(format!(
                        "🔍 [Web Result] \x02{}\x02 | {}",
                        title, clean_link
                    ));
                }
            }
        }

        None
    }

    async fn search_ddg_instant(http: &reqwest::Client, query: &str) -> Option<String> {
        let url = format!(
            "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
            encode_query(query)
        );

        let resp = http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let data = resp.json::<DuckDuckGoInstant>().await.ok()?;
        let heading = data.heading.unwrap_or_default();
        let abstract_text = data.abstract_text.unwrap_or_default();
        let url = data.abstract_url.unwrap_or_default();

        if !abstract_text.is_empty() {
            Some(format!(
                "🔍 [Info] \x02{}\x02 - {} | {}",
                heading, abstract_text, url
            ))
        } else {
            None
        }
    }
}

fn percent_decode(input: &str) -> String {
    let mut result = String::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                result.push(val as char);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

#[async_trait]
impl Plugin for GooglePlugin {
    fn name(&self) -> &'static str {
        "google"
    }

    fn triggers(&self) -> &[&'static str] {
        &["g", "google", "search"]
    }

    fn help(&self) -> &'static str {
        "!g <zoekterm> - Zoekt op het web en toont het beste resultaat"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let query = cmd.args.trim();
        if query.is_empty() {
            return Ok(Some(ctx.locale.t("google_usage").to_string()));
        }

        // 1. Check if Google Custom Search API is configured
        if let (Ok(key), Ok(cx)) = (std::env::var("GOOGLE_API_KEY"), std::env::var("GOOGLE_CSE_ID")) {
            if !key.is_empty() && !cx.is_empty() {
                if let Some(res) = Self::search_google_cse(&ctx.http, &key, &cx, query).await {
                    return Ok(Some(res));
                }
            }
        }

        // 2. HTML web scraping via DuckDuckGo
        if let Some(res) = Self::search_ddg_html(&ctx.http, query).await {
            return Ok(Some(res));
        }

        // 3. Fallback: Instant answers API
        if let Some(res) = Self::search_ddg_instant(&ctx.http, query).await {
            return Ok(Some(res));
        }

        Ok(Some(ctx.locale.tf("google_not_found", &[("query", query)])))
    }
}
