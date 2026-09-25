use super::{CommandEvent, MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use std::sync::OnceLock;

static YT_URL_REGEX: OnceLock<Regex> = OnceLock::new();
static VIDEO_ID_REGEX: OnceLock<Regex> = OnceLock::new();
static DESC_REGEX: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct OEmbedResponse {
    pub title: Option<String>,
    pub author_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YouTubeApiItemSnippet {
    title: Option<String>,
    #[serde(rename = "channelTitle")]
    channel_title: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YouTubeApiItemId {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YouTubeApiItem {
    id: Option<YouTubeApiItemId>,
    snippet: Option<YouTubeApiItemSnippet>,
}

#[derive(Debug, Deserialize)]
struct YouTubeApiResponse {
    items: Option<Vec<YouTubeApiItem>>,
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

pub struct YouTubePlugin;

impl YouTubePlugin {
    fn get_url_regex() -> &'static Regex {
        YT_URL_REGEX.get_or_init(|| {
            Regex::new(r#"(?:https?://)?(?:www\.)?(?:youtube\.com/(?:watch\?v=|shorts/)|youtu\.be/)([a-zA-Z0-9_-]{11})"#)
                .expect("Valid YouTube URL regex")
        })
    }

    fn get_video_id_regex() -> &'static Regex {
        VIDEO_ID_REGEX.get_or_init(|| {
            Regex::new(r#""videoId":"([a-zA-Z0-9_-]{11})""#)
                .expect("Valid Video ID regex")
        })
    }

    async fn fetch_oembed(
        http: &reqwest::Client,
        video_id: &str,
    ) -> Option<(String, String)> {
        let url = format!(
            "https://www.youtube.com/oembed?url=https://www.youtube.com/watch?v={}&format=json",
            video_id
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

        let data = resp.json::<OEmbedResponse>().await.ok()?;
        let title = data.title.unwrap_or_else(|| "YouTube Video".to_string());
        let author = data.author_name.unwrap_or_else(|| "Onbekend".to_string());
        Some((title, author))
    }

    async fn fetch_description(
        http: &reqwest::Client,
        video_id: &str,
    ) -> Option<String> {
        let url = format!("https://www.youtube.com/watch?v={}", video_id);
        let resp = http
            .get(&url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .header("Accept-Language", "nl,en;q=0.9")
            .timeout(std::time::Duration::from_secs(4))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let body = resp.text().await.ok()?;
        let desc_re = DESC_REGEX.get_or_init(|| {
            Regex::new(r#"(?:name="description"|property="og:description")\s+content="([^"]*)""#)
                .expect("Valid Description regex")
        });

        if let Some(caps) = desc_re.captures(&body) {
            if let Some(desc) = caps.get(1) {
                let raw = desc.as_str().replace('\n', " ").replace('\r', "");
                let clean = raw.split_whitespace().collect::<Vec<_>>().join(" ");
                if !clean.is_empty() {
                    let truncated = if clean.chars().count() > 180 {
                        let mut t: String = clean.chars().take(180).collect();
                        t.push_str("...");
                        t
                    } else {
                        clean
                    };
                    return Some(truncated);
                }
            }
        }
        None
    }

    async fn search_via_api(
        http: &reqwest::Client,
        api_key: &str,
        query: &str,
    ) -> Option<(String, String, String, Option<String>)> {
        let encoded = encode_query(query);
        let url = format!(
            "https://www.googleapis.com/youtube/v3/search?part=snippet&type=video&maxResults=1&q={}&key={}",
            encoded, api_key
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

        let data: YouTubeApiResponse = resp.json().await.ok()?;
        let item = data.items?.into_iter().next()?;
        let video_id = item.id?.video_id?;
        let snippet = item.snippet?;
        let title = snippet.title.unwrap_or_else(|| "YouTube Video".to_string());
        let author = snippet.channel_title.unwrap_or_else(|| "Onbekend".to_string());
        let desc = snippet.description.filter(|s| !s.trim().is_empty()).map(|d| {
            let clean = d.split_whitespace().collect::<Vec<_>>().join(" ");
            if clean.chars().count() > 180 {
                let mut t: String = clean.chars().take(180).collect();
                t.push_str("...");
                t
            } else {
                clean
            }
        });

        Some((video_id, title, author, desc))
    }
}

#[async_trait]
impl Plugin for YouTubePlugin {
    fn name(&self) -> &'static str {
        "youtube"
    }

    fn triggers(&self) -> &[&'static str] {
        &["yt", "youtube"]
    }

    fn help(&self) -> &'static str {
        "!yt <zoekterm> - Zoekt naar een video op YouTube of toont video-informatie"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let query = cmd.args.trim();
        if query.is_empty() {
            return Ok(Some(ctx.locale.t("youtube_usage").to_string()));
        }

        let desc_label = ctx.locale.t("youtube_desc_prefix");
        let by_label = ctx.locale.t("youtube_by");

        // 1. Is it already a video URL or video ID?
        if let Some(caps) = Self::get_url_regex().captures(query) {
            if let Some(vid) = caps.get(1) {
                let video_id = vid.as_str();
                if let Some((title, author)) = Self::fetch_oembed(&ctx.http, video_id).await {
                    let desc_str = if let Some(desc) = Self::fetch_description(&ctx.http, video_id).await {
                        format!("\n📝 \x02{}\x02 {}", desc_label, desc)
                    } else {
                        String::new()
                    };

                    return Ok(Some(format!(
                        "▶️ [YouTube] \x02{}\x02 {} \x02{}\x02 | https://youtu.be/{}{}",
                        title, by_label, author, video_id, desc_str
                    )));
                }
            }
        }

        // 2. Try official YouTube Data API v3 if YOUTUBE_API_KEY is configured
        if let Ok(key) = std::env::var("YOUTUBE_API_KEY") {
            let key = key.trim();
            if !key.is_empty() {
                if let Some((video_id, title, author, desc)) = Self::search_via_api(&ctx.http, key, query).await {
                    let desc_str = if let Some(d) = desc {
                        format!("\n📝 \x02{}\x02 {}", desc_label, d)
                    } else {
                        String::new()
                    };

                    return Ok(Some(format!(
                        "▶️ [YouTube API] \x02{}\x02 {} \x02{}\x02 | https://youtu.be/{}{}",
                        title, by_label, author, video_id, desc_str
                    )));
                }
            }
        }

        // 3. Fallback: Search via HTML scraper and oEmbed
        let encoded_query = encode_query(query);
        let search_url = format!("https://www.youtube.com/results?search_query={}", encoded_query);

        let resp = ctx
            .http
            .get(&search_url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .header("Accept-Language", "en-US,en;q=0.9,nl;q=0.8")
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err_msg = if ctx.locale.is_dutch() {
                format!("⚠️ YouTube zoekopdracht mislukt (HTTP Status: {}).", resp.status())
            } else {
                format!("⚠️ YouTube search failed (HTTP Status: {}).", resp.status())
            };
            return Ok(Some(err_msg));
        }

        let body = resp.text().await?;
        if let Some(caps) = Self::get_video_id_regex().captures(&body) {
            if let Some(vid) = caps.get(1) {
                let video_id = vid.as_str();
                if let Some((title, author)) = Self::fetch_oembed(&ctx.http, video_id).await {
                    let desc_str = if let Some(desc) = Self::fetch_description(&ctx.http, video_id).await {
                        format!("\n📝 \x02{}\x02 {}", desc_label, desc)
                    } else {
                        String::new()
                    };

                    return Ok(Some(format!(
                        "▶️ [YouTube] \x02{}\x02 {} \x02{}\x02 | https://youtu.be/{}{}",
                        title, by_label, author, video_id, desc_str
                    )));
                } else {
                    return Ok(Some(format!(
                        "▶️ [YouTube] https://youtu.be/{}",
                        video_id
                    )));
                }
            }
        }

        let not_found_msg = if ctx.locale.is_dutch() {
            format!("🔍 Geen YouTube video's gevonden voor: \"{}\"", query)
        } else {
            format!("🔍 No YouTube videos found for: \"{}\"", query)
        };
        Ok(Some(not_found_msg))
    }

    async fn on_message(
        &self,
        ctx: &PluginContext,
        msg: &MessageEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        // Passieve herkenning van YouTube links in chat
        if let Some(caps) = Self::get_url_regex().captures(&msg.content) {
            if let Some(vid) = caps.get(1) {
                let video_id = vid.as_str();
                if let Some((title, author)) = Self::fetch_oembed(&ctx.http, video_id).await {
                    let desc_str = if let Some(desc) = Self::fetch_description(&ctx.http, video_id).await {
                        format!("\n📝 \x02Omschrijving:\x02 {}", desc)
                    } else {
                        String::new()
                    };

                    return Ok(Some(format!(
                        "▶️ [YouTube] \x02{}\x02 door \x02{}\x02{}",
                        title, author, desc_str
                    )));
                }
            }
        }
        Ok(None)
    }
}
