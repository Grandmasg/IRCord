use super::flood::{chunk_irc_message, RaidGuard, TokenBucketLimiter};
use crate::bridge::{BridgeMessage, Platform, PresenceEvent};
use crate::config::Config;
use crate::utils::sanitizer::sanitize_for_irc;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
    presence_tx: Option<Sender<PresenceEvent>>,
    shutdown_token: CancellationToken,
}

impl IrcClient {
    pub fn new(
        config: Arc<Config>,
        inbound_tx: Sender<BridgeMessage>,
        outbound_rx: Receiver<BridgeMessage>,
        raw_cmd_rx: Receiver<String>,
        presence_tx: Option<Sender<PresenceEvent>>,
        shutdown_token: CancellationToken,
    ) -> Self {
        Self {
            config,
            inbound_tx,
            outbound_rx,
            raw_cmd_rx,
            presence_tx,
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
        let tls = super::tls::TlsSettings::from_env(port);
        let stream = super::tls::connect(server, port, tls).await?;
        if tls.enabled && !tls.verify {
            warn!("IRC_TLS_VERIFY=false: de verbinding is versleuteld maar het servercertificaat wordt niet gecontroleerd.");
        }
        let (reader, mut writer) = tokio::io::split(stream);
        let mut buf_reader = BufReader::new(reader);

        // SASL Credentials uit environment (alleen als wachtwoord niet leeg is)
        let sasl_pass = std::env::var("IRC_SASL_PASS").ok().filter(|s| !s.trim().is_empty());
        let sasl_user = std::env::var("IRC_SASL_USER").unwrap_or_else(|_| nick.to_string());
        let has_sasl = sasl_pass.is_some();
        if has_sasl && !tls.enabled {
            warn!("IRC_SASL_PASS is ingesteld zonder TLS: het wachtwoord gaat in leesbare tekst over de lijn. Zet IRC_USE_TLS=true (poort 6697).");
        }

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
        // Laatst bekend kanaal per nick (voor QUIT-meldingen, die zelf geen kanaal bevatten)
        let mut nick_channels: std::collections::HashMap<String, String> = std::collections::HashMap::new();

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

                    // Aanwezigheid (JOIN/PART/QUIT) doorgeven aan de bridge; we onthouden het laatste kanaal per nick
                    if parts.len() > 1 && matches!(parts[1], "JOIN" | "PART" | "QUIT") {
                        let who = parts[0].trim_start_matches(':').split('!').next().unwrap_or("").to_string();
                        if !who.is_empty() && !who.eq_ignore_ascii_case(nick) {
                            let key = who.to_lowercase();
                            // QUIT heeft de reden als 3e veld, PART als 4e (de reden mag spaties bevatten)
                            let reason = if parts[1] == "QUIT" { raw.splitn(3, ' ').nth(2) } else { raw.splitn(4, ' ').nth(3) }
                                .map(|r| r.trim_start_matches(':').to_string());
                            let event = match parts[1] {
                                "JOIN" if parts.len() > 2 => {
                                    let chan = parts[2].trim_start_matches(':').to_string();
                                    nick_channels.insert(key, chan.clone());
                                    Some(PresenceEvent::Join { nick: who, platform: Platform::Irc, channel: chan })
                                }
                                "PART" if parts.len() > 2 => {
                                    let chan = parts[2].to_string();
                                    nick_channels.remove(&key);
                                    Some(PresenceEvent::Part { nick: who, platform: Platform::Irc, channel: chan, reason })
                                }
                                "QUIT" => {
                                    let chan = nick_channels.remove(&key);
                                    Some(PresenceEvent::Quit { nick: who, platform: Platform::Irc, channel: chan, reason })
                                }
                                _ => None,
                            };
                            if let (Some(ev), Some(tx)) = (event, self.presence_tx.as_ref()) {
                                let _ = tx.try_send(ev);
                            }
                        }
                    }

                    // Kanaalonderwerp gewijzigd (alleen het TOPIC-commando; het 332-antwoord bij joinen negeren we bewust)
                    if parts.len() > 2 && parts[1] == "TOPIC" {
                        if let (Some(tx), Some(text)) = (self.presence_tx.as_ref(), raw.splitn(4, ' ').nth(3)) {
                            let _ = tx.try_send(PresenceEvent::Topic {
                                channel: parts[2].to_string(),
                                topic: text.strip_prefix(':').unwrap_or(text).to_string(),
                            });
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
                                if target_chan.starts_with('#') {
                                    if nick_channels.len() > 5000 {
                                        nick_channels.clear();
                                    }
                                    nick_channels.insert(sender_nick.to_lowercase(), target_chan.to_string());
                                }
                                let bridge_msg = BridgeMessage {
                                    source_platform: Platform::Irc,
                                    source_channel: target_chan.to_string(),
                                    author_name: sender_nick.to_string(),
                                    // Door de server bevestigd account (voor eigenaar/operator-checks)
                                    author_id: tag_account
                                        .as_ref()
                                        .map(|a| format!("{}{}", crate::config::IRC_ACCOUNT_PREFIX, a)),
                                    message_id: None,
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

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    async fn read_until(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>, needle: &str, seen: &mut Vec<String>) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout_at(deadline, reader.read_line(&mut line))
                .await
                .unwrap_or_else(|_| panic!("timeout: wachtte op '{}', zag {:?}", needle, seen))
                .unwrap();
            assert!(n > 0, "verbinding gesloten; wachtte op '{}', zag {:?}", needle, seen);
            let line = line.trim().to_string();
            seen.push(line.clone());
            if line.contains(needle) {
                return;
            }
        }
    }

    /// Volledige handshake + berichtenstroom tegen een nep-IRC-server.
    #[tokio::test]
    async fn handshake_chat_presence_and_raid_guard_against_mock_server() {
        std::env::remove_var("IRC_SASL_PASS");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
        let (out_tx, out_rx) = mpsc::channel(16);
        let (_raw_tx, raw_rx) = mpsc::channel(16);
        let (presence_tx, mut presence_rx) = mpsc::channel(16);
        let token = CancellationToken::new();
        let cfg = Arc::new(crate::config::test_config("", ""));
        let mut client = IrcClient::new(cfg, inbound_tx, out_rx, raw_rx, Some(presence_tx), token.clone());
        let client_task = tokio::spawn(async move { client.connect_and_loop("127.0.0.1", port, "Monkeybot").await });

        let (sock, _) = listener.accept().await.unwrap();
        let (r, mut w) = sock.into_split();
        let mut reader = BufReader::new(r);
        let mut seen = Vec::new();

        // 1. Handshake: account-tag aangevraagd, NICK/USER verstuurd
        read_until(&mut reader, "USER Monkeybot", &mut seen).await;
        assert!(seen.iter().any(|l| l == "CAP REQ :account-tag"), "{:?}", seen);
        assert!(seen.iter().any(|l| l == "NICK Monkeybot"), "{:?}", seen);

        // 2. Welkom (001) => het bridge-kanaal wordt betreden
        w.write_all(b":srv 001 Monkeybot :Welcome\r\n").await.unwrap();
        read_until(&mut reader, "JOIN #test", &mut seen).await;

        // 3. PING/PONG
        w.write_all(b"PING :abc123\r\n").await.unwrap();
        read_until(&mut reader, "PONG :abc123", &mut seen).await;

        // 4. Chatbericht met bevestigd account (account-tag) komt als BridgeMessage binnen
        w.write_all(b"@account=Boss :Boss!u@2001:db8::1 PRIVMSG #test :hallo: wereld 12:30\r\n").await.unwrap();
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), inbound_rx.recv()).await.unwrap().unwrap();
        assert_eq!(msg.author_name, "Boss");
        assert_eq!(msg.author_id.as_deref(), Some("irc-account:Boss"));
        assert_eq!(msg.content, "hallo: wereld 12:30");
        assert_eq!(msg.source_channel, "#test");

        // Zonder tag geen account (nick-spoofing krijgt geen eigenaarrechten)
        w.write_all(b":BossNick!x@y PRIVMSG #test :hoi\r\n").await.unwrap();
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), inbound_rx.recv()).await.unwrap().unwrap();
        assert_eq!(msg.author_id, None);

        // 5. Aanwezigheid: JOIN en QUIT (met onthouden kanaal)
        w.write_all(b":henk!u@h JOIN #test\r\n:henk!u@h QUIT :Ping timeout\r\n").await.unwrap();
        let join = tokio::time::timeout(std::time::Duration::from_secs(5), presence_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(join, PresenceEvent::Join { ref nick, ref channel, .. } if nick == "henk" && channel == "#test"));
        let quit = tokio::time::timeout(std::time::Duration::from_secs(5), presence_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(quit, PresenceEvent::Quit { ref channel, ref reason, .. }
            if channel.as_deref() == Some("#test") && reason.as_deref() == Some("Ping timeout")), "{:?}", quit);

        // 5b. Onderwerpwijziging komt volledig (met spaties) door als Topic-event
        w.write_all(b":henk!u@h TOPIC #test :nieuw onderwerp met spaties
").await.unwrap();
        let topic = tokio::time::timeout(std::time::Duration::from_secs(5), presence_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(topic, PresenceEvent::Topic { ref channel, ref topic } if channel == "#test" && topic == "nieuw onderwerp met spaties"), "{:?}", topic);

        // 6. Uitgaand bericht van de bot wordt als PRIVMSG verstuurd
        out_tx
            .send(BridgeMessage {
                source_platform: Platform::Irc,
                source_channel: "#test".into(),
                author_name: "IRCord".into(),
                author_id: None,
                message_id: None,
                content: "antwoord van de bot".into(),
                reply_to: None,
                is_action: false,
            })
            .await
            .unwrap();
        read_until(&mut reader, "PRIVMSG #test :antwoord van de bot", &mut seen).await;

        // 7. Join-flood (5 joins binnen 1s) => kanaal op +m
        let burst: String = (0..5).map(|i| format!(":raid{i}!u@h JOIN #test\r\n")).collect();
        w.write_all(burst.as_bytes()).await.unwrap();
        read_until(&mut reader, "MODE #test +m", &mut seen).await;

        // 8. Netjes afsluiten
        token.cancel();
        read_until(&mut reader, "QUIT", &mut seen).await;
        assert!(client_task.await.unwrap().is_ok());
    }

    #[test]
    fn privmsg_text_with_ipv6_host_and_colons() {
        assert_eq!(privmsg_text(":nick!u@2001:db8::1 PRIVMSG #c :tijd: 12:30 ok"), Some("tijd: 12:30 ok"));
    }
}
