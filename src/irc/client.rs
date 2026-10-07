use super::flood::{chunk_irc_message, RaidGuard, TokenBucketLimiter};
use crate::bridge::{BridgeMessage, Platform};
use crate::config::Config;
use crate::utils::sanitizer::sanitize_for_irc;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Eenvoudige, veilige Base64-encoder voor SASL PLAIN authenticatie
fn base64_encode(input: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        out.push(CHARSET[(b0 >> 2) as usize] as char);
        out.push(CHARSET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if chunk.len() > 1 {
            out.push(CHARSET[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }

        if chunk.len() > 2 {
            out.push(CHARSET[(b2 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Knipt IRCv3 message tags van een regel en geeft het door de server bevestigde account terug.
fn split_tags(raw: &str) -> (Option<String>, &str) {
    if !raw.starts_with('@') {
        return (None, raw);
    }
    let (tags, rest) = raw.split_once(' ').unwrap_or((raw, ""));
    let account = tags[1..]
        .split(';')
        .find_map(|t| t.strip_prefix("account="))
        .filter(|a| !a.is_empty() && *a != "*")
        .map(str::to_string);
    (account, rest.trim_start())
}

/// Haalt de chattekst uit `:prefix PRIVMSG <target> :tekst` (veilig voor IPv6-hosts met dubbelepunten).
fn privmsg_text(raw: &str) -> Option<&str> {
    raw.splitn(4, ' ').nth(3).map(|rest| rest.strip_prefix(':').unwrap_or(rest))
}

pub struct IrcClient {
    config: Arc<Config>,
    inbound_tx: Sender<BridgeMessage>,
    outbound_rx: Receiver<BridgeMessage>,
    raw_cmd_rx: Receiver<String>,
    shutdown_token: CancellationToken,
}

impl IrcClient {
    pub fn new(
        config: Arc<Config>,
        inbound_tx: Sender<BridgeMessage>,
        outbound_rx: Receiver<BridgeMessage>,
        raw_cmd_rx: Receiver<String>,
        shutdown_token: CancellationToken,
    ) -> Self {
        Self {
            config,
            inbound_tx,
            outbound_rx,
            raw_cmd_rx,
            shutdown_token,
        }
    }

    pub async fn run(mut self) {
        let server = std::env::var("IRC_SERVER").unwrap_or_else(|_| "irc.libera.chat".into());
        let port: u16 = std::env::var("IRC_PORT")
            .unwrap_or_else(|_| "6667".into())
            .parse()
            .unwrap_or(6667);
        let nick = std::env::var("IRC_NICK").unwrap_or_else(|_| "IRCordBot".into());

        info!("IRC Task gestart. Verbinden met {}:{} (Nick: {})...", server, port, nick);

        loop {
            if self.shutdown_token.is_cancelled() {
                break;
            }

            match self.connect_and_loop(&server, port, &nick).await {
                Ok(_) => info!("IRC sessie beëindigd."),
                Err(e) => {
                    warn!("IRC verbinding verbroken: {}. Herverbinden over 5 seconden...", e);
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {},
                _ = self.shutdown_token.cancelled() => {
                    break;
                }
            }
        }

        info!("IRC Task netjes afgesloten.");
    }

    async fn connect_and_loop(&mut self, server: &str, port: u16, nick: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = format!("{}:{}", server, port);
        let stream = TcpStream::connect(&addr).await?;
        let (reader, mut writer) = tokio::io::split(stream);
        let mut buf_reader = BufReader::new(reader);

        // SASL Credentials uit environment (alleen als wachtwoord niet leeg is)
        let sasl_pass = std::env::var("IRC_SASL_PASS").ok().filter(|s| !s.trim().is_empty());
        let sasl_user = std::env::var("IRC_SASL_USER").unwrap_or_else(|_| nick.to_string());
        let has_sasl = sasl_pass.is_some();

        if has_sasl {
            info!("IRC verbinding gestart met IRCv3 CAP & SASL PLAIN handshake...");
            // account-tag laat de server per bericht het geverifieerde account meesturen (eigenaar-/operatorcheck)
            writer.write_all(b"CAP LS 302\r\nCAP REQ :sasl\r\nCAP REQ :account-tag\r\n").await?;
        } else {
            // Reguliere handshake; account-tag aanvragen en de CAP-onderhandeling direct afsluiten
            writer.write_all(format!("CAP REQ :account-tag\r\nCAP END\r\nNICK {}\r\nUSER {} 0 * :IRCord Hybrid Bot\r\n", nick, nick).as_bytes()).await?;
        }

        let mut limiter = TokenBucketLimiter::new(self.config.moderation.irc_flood_delay_ms);
        let mut line_buf = String::new();

        // Anti-raid: bij een join-flood zetten we het kanaal tijdelijk op +m en halen dat daarna weer weg
        let mut raid_guard = RaidGuard::new(
            self.config.moderation.raid_threshold_joins_per_sec,
            std::time::Duration::from_secs(1),
        );
        let raid_mute = std::time::Duration::from_secs(self.config.moderation.raid_mute_duration_sec);
        let mut unmute_at: std::collections::HashMap<String, std::time::Instant> = std::collections::HashMap::new();
        let mut raid_tick = tokio::time::interval(std::time::Duration::from_secs(2));

        loop {
            line_buf.clear();

            tokio::select! {
                // Inkomende IRC data
                read_res = buf_reader.read_line(&mut line_buf) => {
                    let bytes = read_res?;
                    if bytes == 0 {
                        return Err("IRC Socket gesloten door remote host".into());
                    }

                    let mut raw = line_buf.trim();
                    if raw.is_empty() {
                        continue;
                    }

                    // IRCv3 message tags (`@account=foo;time=... :prefix COMMAND ...`) los knippen
                    let (tag_account, rest) = split_tags(raw);
                    raw = rest;
                    if raw.is_empty() {
                        continue;
                    }

                    // Ping-Pong keepalive (ondersteunt zowel 'PING token' als 'PING :token')
                    if raw.starts_with("PING") {
                        let token = raw.split_whitespace().nth(1).unwrap_or("").trim_start_matches(':');
                        writer.write_all(format!("PONG :{}\r\n", token).as_bytes()).await?;
                        continue;
                    }

                    // IRCv3 SASL CAP handshake afhandeling
                    if raw.contains("CAP") && raw.contains("ACK") && raw.contains("sasl") {
                        info!("Server ondersteunt SASL. Verzenden AUTHENTICATE PLAIN...");
                        writer.write_all(b"AUTHENTICATE PLAIN\r\n").await?;
                        continue;
                    }

                    if raw.contains("CAP") && raw.contains("NAK") && raw.contains("sasl") {
                        warn!("Server weigert CAP sasl. Doorgaan met directe login...");
                        writer.write_all(format!("CAP END\r\nNICK {}\r\nUSER {} 0 * :IRCord Hybrid Bot\r\n", nick, nick).as_bytes()).await?;
                        continue;
                    }

                    if raw.starts_with("AUTHENTICATE +") || raw.starts_with("AUTHENTICATE :+") {
                        if let Some(ref pass) = sasl_pass {
                            info!("SASL challenge ontvangen. Verzenden geëncodeerde credentials...");
                            let mut payload = Vec::new();
                            payload.push(0);
                            payload.extend_from_slice(sasl_user.as_bytes());
                            payload.push(0);
                            payload.extend_from_slice(pass.as_bytes());
                            let encoded = base64_encode(&payload);
                            writer.write_all(format!("AUTHENTICATE {}\r\n", encoded).as_bytes()).await?;
                        }
                        continue;
                    }

                    // SASL Succes (903) of Fout (904, 905) of Nick in use (433)
                    let parts: Vec<&str> = raw.split_whitespace().collect();
                    if parts.len() > 1 {
                        let numeric = parts[1];
                        if numeric == "903" {
                            info!("SASL PLAIN authenticatie succesvol bevestigd door server!");
                            writer.write_all(format!("CAP END\r\nNICK {}\r\nUSER {} 0 * :IRCord Hybrid Bot\r\n", nick, nick).as_bytes()).await?;
                            continue;
                        } else if numeric == "904" || numeric == "905" {
                            warn!("SASL authenticatie mislukt code ({}). Doorgaan als unauthenticated...", numeric);
                            writer.write_all(format!("CAP END\r\nNICK {}\r\nUSER {} 0 * :IRCord Hybrid Bot\r\n", nick, nick).as_bytes()).await?;
                            continue;
                        } else if numeric == "433" {
                            let alt_nick = format!("{}_", nick);
                            warn!("IRC Nick '{}' reeds in gebruik! Proberen met alternatief: '{}'...", nick, alt_nick);
                            writer.write_all(format!("NICK {}\r\n", alt_nick).as_bytes()).await?;
                            continue;
                        }
                    }

                    // 001 Welkomstbericht: join kanalen (direct geauthenticeerd!)
                    if parts.len() > 1 && parts[1] == "001" {
                        info!("Succesvol geauthenticeerd op IRC server!");
                        for mapping in &self.config.channels {
                            info!("IRC kanaal joinen: {}", mapping.irc_channel);
                            writer.write_all(format!("JOIN {}\r\n", mapping.irc_channel).as_bytes()).await?;
                        }

                        let admin_chan = &self.config.general.admin_channel_irc;
                        if !admin_chan.is_empty() && !self.config.channels.iter().any(|c| c.irc_channel.eq_ignore_ascii_case(admin_chan)) {
                            info!("IRC admin/log kanaal joinen: {}", admin_chan);
                            writer.write_all(format!("JOIN {}\r\n", admin_chan).as_bytes()).await?;
                        }
                    }

                    // JOIN-flood detectie (alleen joins van anderen)
                    if parts.len() > 2 && parts[1] == "JOIN" {
                        let joiner = parts[0].trim_start_matches(':').split('!').next().unwrap_or("");
                        let chan = parts[2].trim_start_matches(':');
                        if !joiner.eq_ignore_ascii_case(nick)
                            && chan.starts_with('#')
                            && raid_guard.record_join(chan, std::time::Instant::now())
                            && !unmute_at.contains_key(&chan.to_lowercase())
                        {
                            warn!("🚨 Join-flood gedetecteerd in {}: kanaal {}s op +m", chan, raid_mute.as_secs());
                            writer.write_all(format!("MODE {} +m\r\n", chan).as_bytes()).await?;
                            unmute_at.insert(chan.to_lowercase(), std::time::Instant::now() + raid_mute);
                        }
                    }

                    // KICK afhandeling: Auto-rejoin bij kick van de bot
                    if parts.len() > 3 && parts[1] == "KICK" {
                        let kicked_chan = parts[2];
                        let kicked_nick = parts[3];
                        if kicked_nick.eq_ignore_ascii_case(nick) || kicked_nick.eq_ignore_ascii_case(&format!("{}_", nick)) {
                            warn!("Bot werd gekickt uit {}! Auto-rejoin over 3 seconden...", kicked_chan);
                            let chan_clone = kicked_chan.to_string();
                            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                            writer.write_all(format!("JOIN {}\r\n", chan_clone).as_bytes()).await?;
                            continue;
                        }
                    }

                    // INVITE afhandeling: automatische join wanneer uitgenodigd
                    if parts.len() > 3 && parts[1] == "INVITE" {
                        let invited_chan = parts[3].trim_start_matches(':');
                        let inviter = parts[0].trim_start_matches(':').split('!').next().unwrap_or("onbekend");
                        info!("Bot uitgenodigd voor kanaal {} door {}! Auto-joinen...", invited_chan, inviter);
                        writer.write_all(format!("JOIN {}\r\n", invited_chan).as_bytes()).await?;
                        continue;
                    }

                    // PRIVMSG afhandeling
                    if parts.len() > 3 && parts[1] == "PRIVMSG" {
                        let prefix = parts[0].trim_start_matches(':');
                        let sender_nick = prefix.split('!').next().unwrap_or("onbekend");
                        let target_chan = parts[2];

                        // Parse de eigenlijke chattekst: alles na `:prefix PRIVMSG <target> :`
                        // (niet op de eerste ':' zoeken; hostnamen kunnen IPv6-dubbelepunten bevatten)
                        let text_opt = privmsg_text(raw);
                        if let Some(text) = text_opt {
                            // CTCP Queries afhandelen (bijv. VERSION, PING, TIME)
                            let trimmed_text = text.trim();
                            if trimmed_text.starts_with('\x01') && trimmed_text.ends_with('\x01') {
                                let ctcp_content = trimmed_text.trim_matches('\x01').trim();
                                let mut ctcp_parts = ctcp_content.splitn(2, ' ');
                                let ctcp_tag = ctcp_parts.next().unwrap_or("").to_uppercase();
                                let ctcp_arg = ctcp_parts.next().unwrap_or("");

                                match ctcp_tag.as_str() {
                                    "VERSION" => {
                                        info!("CTCP VERSION ontvangen van {}", sender_nick);
                                        let reply = format!("NOTICE {} :\x01VERSION IRCord v2.2.0 Hybrid Bridge & Bot (Rust) by Grandmasg\x01\r\n", sender_nick);
                                        writer.write_all(reply.as_bytes()).await?;
                                        continue;
                                    }
                                    "PING" => {
                                        info!("CTCP PING ontvangen van {}", sender_nick);
                                        let reply = format!("NOTICE {} :\x01PING {}\x01\r\n", sender_nick, ctcp_arg);
                                        writer.write_all(reply.as_bytes()).await?;
                                        continue;
                                    }
                                    "TIME" => {
                                        info!("CTCP TIME ontvangen van {}", sender_nick);
                                        let now_str = chrono::Utc::now().to_rfc2822();
                                        let reply = format!("NOTICE {} :\x01TIME {}\x01\r\n", sender_nick, now_str);
                                        writer.write_all(reply.as_bytes()).await?;
                                        continue;
                                    }
                                    "ACTION" => {
                                        // Emote (/me), doorlaten naar bridge!
                                    }
                                    _ => {
                                        continue;
                                    }
                                }
                            }

                            if !sender_nick.eq_ignore_ascii_case(nick) {
                                let bridge_msg = BridgeMessage {
                                    source_platform: Platform::Irc,
                                    source_channel: target_chan.to_string(),
                                    author_name: sender_nick.to_string(),
                                    // Door de server bevestigd account (voor eigenaar/operator-checks)
                                    author_id: tag_account
                                        .as_ref()
                                        .map(|a| format!("{}{}", crate::config::IRC_ACCOUNT_PREFIX, a)),
                                    content: text.to_string(),
                                    reply_to: None,
                                    is_action: text.starts_with("\x01ACTION"),
                                };

                                let _ = self.inbound_tx.send(bridge_msg).await;
                            }
                        }
                    }
                }

                // Uitgaande ruwe IRC commando's (KICK, MODE, TOPIC, JOIN etc.)
                raw_cmd = self.raw_cmd_rx.recv() => {
                    if let Some(cmd) = raw_cmd {
                        // Nooit CR/LF/NUL doorlaten: voorkomt injectie van extra IRC-commando's
                        let clean_owned = cmd.replace(['\r', '\n', '\0'], " ");
                        let clean = clean_owned.trim();
                        if !clean.is_empty() {
                            limiter.wait_for_slot().await;
                            let line = format!("{}\r\n", clean);
                            let _ = writer.write_all(line.as_bytes()).await;
                        }
                    }
                }

                // Uitgaande berichten vanuit bridge of plugins naar IRC
                out_msg = self.outbound_rx.recv() => {
                    if let Some(msg) = out_msg {
                        let safe_chan = sanitize_for_irc(&msg.source_channel);
                        let chunks = chunk_irc_message(&msg.content, self.config.moderation.irc_line_max_bytes);
                        for chunk in chunks {
                            let safe_chunk = sanitize_for_irc(&chunk);
                            if safe_chunk.is_empty() {
                                continue;
                            }
                            limiter.wait_for_slot().await;
                            let line = format!("PRIVMSG {} :{}\r\n", safe_chan, safe_chunk);
                            writer.write_all(line.as_bytes()).await?;
                        }
                    }
                }

                // Verlopen raid-mutes opheffen
                _ = raid_tick.tick() => {
                    let now = std::time::Instant::now();
                    let due: Vec<String> = unmute_at.iter().filter(|(_, t)| **t <= now).map(|(c, _)| c.clone()).collect();
                    for chan in due {
                        unmute_at.remove(&chan);
                        info!("Raid-mute verlopen voor {}: -m", chan);
                        writer.write_all(format!("MODE {} -m\r\n", chan).as_bytes()).await?;
                    }
                }

                // Shutdown signaal
                _ = self.shutdown_token.cancelled() => {
                    info!("IRC client sluit af: verzenden QUIT...");
                    let _ = writer.write_all(b"QUIT :IRCord daemon gracefully shutting down...\r\n").await;
                    break;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_give_verified_account() {
        let (acct, rest) = split_tags("@time=2026-10-07T12:00:00.000Z;account=Boss :nick!u@h PRIVMSG #c :hoi");
        assert_eq!(acct.as_deref(), Some("Boss"));
        assert_eq!(rest, ":nick!u@h PRIVMSG #c :hoi");
        assert_eq!(split_tags("@account=* :n!u@h PRIVMSG #c :x").0, None);
        assert_eq!(split_tags(":n!u@h PRIVMSG #c :x").0, None);
    }

    #[test]
    fn privmsg_text_with_ipv6_host_and_colons() {
        assert_eq!(privmsg_text(":nick!u@2001:db8::1 PRIVMSG #c :tijd: 12:30 ok"), Some("tijd: 12:30 ok"));
    }
}
