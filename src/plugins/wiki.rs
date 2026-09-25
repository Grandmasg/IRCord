use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;

pub struct WikipediaPlugin;

#[derive(Deserialize)]
struct WikiSummary {
    title: Option<String>,
    extract: Option<String>,
    content_urls: Option<WikiContentUrls>,
    #[serde(rename = "type")]
    page_type: Option<String>,
}

#[derive(Deserialize)]
struct WikiContentUrls {
    desktop: Option<WikiDesktopUrl>,
}

#[derive(Deserialize)]
struct WikiDesktopUrl {
    page: Option<String>,
}

#[async_trait]
impl Plugin for WikipediaPlugin {
    fn name(&self) -> &'static str {
        "wiki"
    }

    fn triggers(&self) -> &[&'static str] {
        &["wiki", "wkp", "wikipedia"]
    }

    fn help(&self) -> &'static str {
        "!wiki <zoekterm> - Zoekt een samenvatting op Wikipedia (NL met EN fallback)"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let term = cmd.args.trim();
        if term.is_empty() {
            return Ok(Some("Gebruik: !wiki <zoekterm>".into()));
        }

        // Ondersteun expliciete taalselectie: !wiki en <zoekterm> of !wiki nl <zoekterm>
        let (lang, query) = if let Some((first, rest)) = term.split_once(' ') {
            if first.eq_ignore_ascii_case("en") || first.eq_ignore_ascii_case("nl") || first.eq_ignore_ascii_case("de") {
                (first.to_lowercase(), rest.trim())
            } else {
                ("nl".to_string(), term)
            }
        } else {
            ("nl".to_string(), term)
        };

        if query.is_empty() {
            return Ok(Some("Gebruik: !wiki [taal] <zoekterm>".into()));
        }

        // Probeer eerst gekozen taal (standaard NL)
        let encoded = urlencoding_simple(query);
        let url = format!("https://{}.wikipedia.org/api/rest_v1/page/summary/{}", lang, encoded);

        let resp = ctx
            .http
            .get(&url)
            .header("User-Agent", "IRCordBot/1.0 (bot; contact@example.com)")
            .send()
            .await?;

        // Fallback naar Engels als NL 404 geeft
        let (final_resp, used_lang) = if resp.status().as_u16() == 404 && lang == "nl" {
            let en_url = format!("https://en.wikipedia.org/api/rest_v1/page/summary/{}", encoded);
            let en_resp = ctx
                .http
                .get(&en_url)
                .header("User-Agent", "IRCordBot/1.0 (bot; contact@example.com)")
                .send()
                .await?;
            (en_resp, "en".to_string())
        } else {
            (resp, lang)
        };

        if !final_resp.status().is_success() {
            return Ok(Some(format!(
                "📖 Geen Wikipedia-artikel gevonden voor '{}'.",
                query
            )));
        }

        let summary: WikiSummary = final_resp.json().await?;
        let title = summary.title.unwrap_or_else(|| query.to_string());
        let extract = summary.extract.unwrap_or_default();
        let page_url = summary
            .content_urls
            .and_then(|u| u.desktop)
            .and_then(|d| d.page)
            .unwrap_or_else(|| format!("https://{}.wikipedia.org/wiki/{}", used_lang, encoded));

        if extract.is_empty() {
            return Ok(Some(format!("📖 [Wikipedia ({})] {} | {}", used_lang.to_uppercase(), title, page_url)));
        }

        // Kort extract in tot max 240 tekens voor IRC overzichtelijkheid
        let short_extract = if extract.chars().count() > 240 {
            let mut cut: String = extract.chars().take(237).collect();
            cut.push_str("...");
            cut
        } else {
            extract
        };

        let note = if summary.page_type.as_deref() == Some("disambiguation") {
            " (Doorverwijspagina)"
        } else {
            ""
        };

        Ok(Some(format!(
            "📖 [Wikipedia ({})] {}{}: {} | {}",
            used_lang.to_uppercase(),
            title,
            note,
            short_extract,
            page_url
        )))
    }
}

fn urlencoding_simple(s: &str) -> String {
    s.trim().replace(' ', "_")
}
