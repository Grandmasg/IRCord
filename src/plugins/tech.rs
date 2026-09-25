use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;

pub struct TechPlugin;

#[derive(Deserialize, Debug)]
struct GhLicense {
    spdx_id: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GhRepoResponse {
    full_name: String,
    description: Option<String>,
    stargazers_count: u64,
    forks_count: u64,
    open_issues_count: u64,
    html_url: String,
    license: Option<GhLicense>,
}

#[derive(Deserialize, Debug)]
struct GhReleaseResponse {
    tag_name: String,
    published_at: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OsvSeverity {
    #[serde(rename = "type")]
    severity_type: String,
    score: String,
}

#[derive(Deserialize, Debug)]
struct OsvResponse {
    id: String,
    summary: Option<String>,
    details: Option<String>,
    severity: Option<Vec<OsvSeverity>>,
    aliases: Option<Vec<String>>,
}

#[async_trait]
impl Plugin for TechPlugin {
    fn name(&self) -> &'static str {
        "tech"
    }

    fn triggers(&self) -> &[&'static str] {
        &["gh", "github", "cve", "security"]
    }

    fn help(&self) -> &'static str {
        "!gh <owner/repo> - Inspect GitHub repository & latest release | !cve <CVE-ID> - Look up vulnerability"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trigger = cmd.trigger.as_str();

        match trigger {
            "gh" | "github" => self.handle_github(ctx, cmd).await,
            "cve" | "security" => self.handle_cve(ctx, cmd).await,
            _ => Ok(None),
        }
    }
}

impl TechPlugin {
    async fn handle_github(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let input = cmd.args.trim().trim_start_matches("https://github.com/").trim_matches('/');
        if input.is_empty() || !input.contains('/') {
            return Ok(Some("🐙 Usage: !gh <owner/repo> (e.g. !gh rust-lang/rust)".into()));
        }

        let parts: Vec<&str> = input.split('/').collect();
        if parts.len() < 2 {
            return Ok(Some("⚠️ Invalid repository format. Use: <owner>/<repo>".into()));
        }
        let owner = parts[0];
        let repo = parts[1];

        let repo_url = format!("https://api.github.com/repos/{}/{}", owner, repo);
        let mut req = ctx.http.get(&repo_url).header("User-Agent", "IRCord-Bot");

        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            if !token.trim().is_empty() {
                req = req.header("Authorization", format!("Bearer {}", token.trim()));
            }
        }

        let resp = req.timeout(std::time::Duration::from_secs(6)).send().await;

        match resp {
            Ok(r) if r.status().as_u16() == 404 => {
                Ok(Some(format!("🐙 [GitHub] Repository '{}/{}' not found.", owner, repo)))
            }
            Ok(r) if r.status().is_success() => {
                let repo_data: GhRepoResponse = r.json().await?;

                // Also check latest release
                let release_url = format!("https://api.github.com/repos/{}/{}/releases/latest", owner, repo);
                let mut rel_req = ctx.http.get(&release_url).header("User-Agent", "IRCord-Bot");
                if let Ok(token) = std::env::var("GITHUB_TOKEN") {
                    if !token.trim().is_empty() {
                        rel_req = rel_req.header("Authorization", format!("Bearer {}", token.trim()));
                    }
                }
                let rel_tag = if let Ok(rel_resp) = rel_req.timeout(std::time::Duration::from_secs(3)).send().await {
                    if rel_resp.status().is_success() {
                        rel_resp.json::<GhReleaseResponse>().await.ok().map(|rel| {
                            let date_str = rel.published_at.map(|d| d.chars().take(10).collect::<String>()).unwrap_or_default();
                            if date_str.is_empty() {
                                rel.tag_name
                            } else {
                                format!("{} ({})", rel.tag_name, date_str)
                            }
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                let desc = repo_data.description.unwrap_or_else(|| "No description provided.".into());
                let short_desc = if desc.len() > 100 {
                    format!("{}...", &desc[..100])
                } else {
                    desc
                };

                let license_str = repo_data.license.and_then(|l| l.spdx_id).unwrap_or_else(|| "No license".into());
                let release_str = rel_tag.map(|t| format!(" | Release: \x02{}\x02", t)).unwrap_or_default();

                Ok(Some(format!(
                    "🐙 [GitHub] \x02{}\x02: {} | ⭐ {} | 🍴 {} | Issues: {} | License: {}{} - {}",
                    repo_data.full_name,
                    short_desc,
                    repo_data.stargazers_count,
                    repo_data.forks_count,
                    repo_data.open_issues_count,
                    license_str,
                    release_str,
                    repo_data.html_url
                )))
            }
            Ok(r) => Ok(Some(format!("⚠️ GitHub API returned status code: {}", r.status()))),
            Err(e) => Ok(Some(format!("❌ Failed to reach GitHub API: {}", e))),
        }
    }

    async fn handle_cve(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut query = cmd.args.trim().to_uppercase();
        if query.is_empty() {
            return Ok(Some("🛡️ Usage: !cve <CVE-ID> (e.g. !cve CVE-2024-3094)".into()));
        }

        if !query.starts_with("CVE-") && query.chars().all(|c| c.is_ascii_digit() || c == '-') {
            query = format!("CVE-{}", query);
        }

        let osv_url = format!("https://api.osv.dev/v1/vulns/{}", query);
        let resp = ctx.http.get(&osv_url).timeout(std::time::Duration::from_secs(6)).send().await;

        match resp {
            Ok(r) if r.status().as_u16() == 404 => {
                Ok(Some(format!("🛡️ [CVE] Vulnerability '{}' not found in OSV database.", query)))
            }
            Ok(r) if r.status().is_success() => {
                let osv: OsvResponse = r.json().await?;

                let summary = osv.summary
                    .or(osv.details)
                    .unwrap_or_else(|| "No detailed description available.".into());

                let short_summary = if summary.len() > 140 {
                    format!("{}...", &summary[..140].replace('\n', " "))
                } else {
                    summary.replace('\n', " ")
                };

                let severity_str = if let Some(sevs) = osv.severity {
                    if let Some(first) = sevs.first() {
                        format!(" | Severity: {}", first.score)
                    } else {
                        "".into()
                    }
                } else {
                    "".into()
                };

                let link = format!("https://osv.dev/vulnerability/{}", osv.id);

                Ok(Some(format!(
                    "🛡️ [Security/CVE] \x02{}\x02: {}{}\n🔗 More info: {}",
                    osv.id, short_summary, severity_str, link
                )))
            }
            Ok(r) => Ok(Some(format!("⚠️ OSV.dev API returned status code: {}", r.status()))),
            Err(e) => Ok(Some(format!("❌ Failed to contact security database: {}", e))),
        }
    }

    pub fn normalize_cve_query(input: &str) -> String {
        let mut query = input.trim().to_uppercase();
        if !query.starts_with("CVE-") && query.chars().all(|c| c.is_ascii_digit() || c == '-') {
            query = format!("CVE-{}", query);
        }
        query
    }

    pub fn parse_gh_repo(input: &str) -> Option<(String, String)> {
        let cleaned = input.trim().trim_start_matches("https://github.com/").trim_matches('/');
        let parts: Vec<&str> = cleaned.split('/').collect();
        if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            Some((parts[0].to_string(), parts[1].to_string()))
        } else {
            None
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_cve_normalization() {
        assert_eq!(TechPlugin::normalize_cve_query("2024-3094"), "CVE-2024-3094");
        assert_eq!(TechPlugin::normalize_cve_query("cve-2024-3094"), "CVE-2024-3094");
        assert_eq!(TechPlugin::normalize_cve_query("CVE-2024-3094"), "CVE-2024-3094");
    }

    #[test]
    fn test_gh_repo_parsing() {
        assert_eq!(TechPlugin::parse_gh_repo("rust-lang/rust"), Some(("rust-lang".into(), "rust".into())));
        assert_eq!(TechPlugin::parse_gh_repo("https://github.com/tokio-rs/tokio/"), Some(("tokio-rs".into(), "tokio".into())));
        assert_eq!(TechPlugin::parse_gh_repo("invalid"), None);
    }
}

