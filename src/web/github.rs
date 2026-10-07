use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub struct GitHubWebhookValidator;

impl GitHubWebhookValidator {
    /// Valideert de X-Hub-Signature-256 header tegen het geconfigureerde geheim
    pub fn verify_signature(secret: &str, payload: &[u8], signature_header: &str) -> bool {
        let expected_prefix = "sha256=";
        if !signature_header.starts_with(expected_prefix) {
            return false;
        }

        let hex_signature = &signature_header[expected_prefix.len()..];
        let Ok(signature_bytes) = hex::decode(hex_signature) else {
            return false;
        };

        let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
            return false;
        };

        mac.update(payload);
        mac.verify_slice(&signature_bytes).is_ok()
    }
}

/// Zet een GitHub webhook-event om naar een korte chatmelding. `None` = event negeren.
pub fn format_event(event: &str, v: &serde_json::Value, dutch: bool) -> Option<String> {
    let repo = v["repository"]["name"].as_str().unwrap_or("repo");
    let sender = v["sender"]["login"].as_str().unwrap_or("iemand");
    let one_line = |s: &str| s.lines().next().unwrap_or("").chars().take(120).collect::<String>();

    match event {
        "push" => {
            let commits = v["commits"].as_array().map(|c| c.len()).unwrap_or(0);
            if commits == 0 || v["deleted"].as_bool().unwrap_or(false) {
                return None;
            }
            let branch = v["ref"].as_str().unwrap_or("").trim_start_matches("refs/heads/");
            let head = one_line(v["head_commit"]["message"].as_str().unwrap_or(""));
            let pusher = v["pusher"]["name"].as_str().unwrap_or(sender);
            Some(if dutch {
                format!("🔨 [{repo}] {pusher} pushte {commits} commit(s) naar {branch}: {head}")
            } else {
                format!("🔨 [{repo}] {pusher} pushed {commits} commit(s) to {branch}: {head}")
            })
        }
        "pull_request" => {
            let action = v["action"].as_str()?;
            let merged = v["pull_request"]["merged"].as_bool().unwrap_or(false);
            let verb = match (action, merged, dutch) {
                ("opened", _, true) => "geopend",
                ("opened", _, false) => "opened",
                ("reopened", _, true) => "heropend",
                ("reopened", _, false) => "reopened",
                ("closed", true, true) => "gemerged",
                ("closed", true, false) => "merged",
                ("closed", false, true) => "gesloten",
                ("closed", false, false) => "closed",
                _ => return None,
            };
            let n = v["pull_request"]["number"].as_u64()?;
            let title = one_line(v["pull_request"]["title"].as_str().unwrap_or(""));
            Some(format!("🔀 [{repo}] PR #{n} {verb} ({sender}): {title}"))
        }
        "issues" => {
            let action = v["action"].as_str()?;
            let verb = match (action, dutch) {
                ("opened", true) => "geopend",
                ("opened", false) => "opened",
                ("reopened", true) => "heropend",
                ("reopened", false) => "reopened",
                ("closed", true) => "gesloten",
                ("closed", false) => "closed",
                _ => return None,
            };
            let n = v["issue"]["number"].as_u64()?;
            let title = one_line(v["issue"]["title"].as_str().unwrap_or(""));
            Some(format!("🐛 [{repo}] Issue #{n} {verb} ({sender}): {title}"))
        }
        "release" if v["action"].as_str() == Some("published") => {
            let tag = v["release"]["tag_name"].as_str().unwrap_or("?");
            let name = one_line(v["release"]["name"].as_str().unwrap_or(""));
            Some(if dutch {
                format!("🚀 [{repo}] Release {tag} gepubliceerd: {name}")
            } else {
                format!("🚀 [{repo}] Release {tag} published: {name}")
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signature_roundtrip() {
        let secret = "s3cret";
        let body = b"{\"a\":1}";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(GitHubWebhookValidator::verify_signature(secret, body, &sig));
        assert!(!GitHubWebhookValidator::verify_signature("fout", body, &sig));
        assert!(!GitHubWebhookValidator::verify_signature(secret, body, "sha256=00"));
    }

    #[test]
    fn formats_events() {
        let push = json!({"ref":"refs/heads/main","commits":[{}, {}],"head_commit":{"message":"fix: bug\n\nlang"},
            "pusher":{"name":"Henk"},"repository":{"name":"ircord"},"sender":{"login":"henk"}});
        assert_eq!(format_event("push", &push, true).unwrap(), "🔨 [ircord] Henk pushte 2 commit(s) naar main: fix: bug");
        let pr = json!({"action":"closed","pull_request":{"number":5,"title":"Nieuw","merged":true},
            "repository":{"name":"ircord"},"sender":{"login":"henk"}});
        assert!(format_event("pull_request", &pr, false).unwrap().contains("PR #5 merged"));
        assert!(format_event("pull_request", &json!({"action":"labeled"}), true).is_none());
        assert!(format_event("star", &json!({}), true).is_none());
    }
}
