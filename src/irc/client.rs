use super::flood::{chunk_irc_message, TokenBucketLimiter};
use crate::bridge::{BridgeMessage, Platform};
use crate::config::Config;
use crate::utils::sanitizer::sanitize_for_irc;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Eenvoudige, veilige Base64-encoder voor SASL PLAIN authenticatie
fn base64_encode(input: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
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

pub struct IrcClient {
    config: Arc<Config>,
    inbound_tx: Sender<BridgeMessage>,
    outbound_rx: Receiver<BridgeMessage>,
    shutdown_token: CancellationToken,
}

impl IrcClient {
    pub fn new(
        config: Arc<Config>,
        inbound_tx: Sender<BridgeMessage>,
        outbound_rx: Receiver<BridgeMessage>,
        shutdown_token: CancellationToken,
    ) -> Self {
        Self {
            config,
            inbound_tx,
            outbound_rx,
            shutdown_token,
        }
    }

    pub async fn run(mut self) {
        let server = std::env::var("IRC_SERVER").unwrap_or_else(|_| "irc.libera.chat".into());
        let port: u16 = std::env::var("IRC_PORT")
            .unwrap_or_else(|_| "6697".into())
            .parse()
            .unwrap_or(6697);
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

        // SASL Credentials uit environment
        let sasl_pass = std::env::var("IRC_SASL_PASS").ok();
        let sasl_user = std::env::var("IRC_SASL_USER").unwrap_or_else(|_| nick.to_string());
        let has_sasl = sasl_pass.is_some();

        if has_sasl {
            info!("IRC verbinding gestart met IRCv3 CAP & SASL PLAIN handshake...");
            writer.write_all(b"CAP LS 302\r\nCAP REQ :sasl\r\n").await?;
        } else {
            // Reguliere directe handshake
            writer.write_all(format!("NICK {}\r\nUSER {} 0 * :IRCord Hybrid Bot\r\n", nick, nick).as_bytes()).await?;
        }

        let mut limiter = TokenBucketLimiter::new(self.config.moderation.irc_flood_delay_ms);
        let mut line_buf = String::new();

        loop {
            line_buf.clear();

            tokio::select! {
                // Inkomende IRC data
                read_res = buf_reader.read_line(&mut line_buf) => {
                    let bytes = read_res?;
                    if bytes == 0 {
                        return Err("IRC Socket gesloten door remote host".into());
                    }

                    let raw = line_buf.trim();
                    if raw.is_empty() {
                        continue;
                    }

                    // Ping-Pong keepalive
                    if raw.starts_with("PING") {
                        let token = raw.split_whitespace().nth(1).unwrap_or("");
                        writer.write_all(format!("PONG {}\r\n", token).as_bytes()).await?;
                        continue;
                    }

                    // IRCv3 SASL CAP handshake afhandeling
                    if raw.contains("CAP") && raw.contains("ACK") && raw.contains("sasl") {
                        info!("Server ondersteunt SASL. Verzenden AUTHENTICATE PLAIN...");
                        writer.write_all(b"AUTHENTICATE PLAIN\r\n").await?;
                        continue;
                    }

                    if raw.starts_with("AUTHENTICATE +") {
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

                    // SASL Succes (903) of Fout (904, 905)
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

                    // PRIVMSG afhandeling
                    if parts.len() > 3 && parts[1] == "PRIVMSG" {
                        let prefix = parts[0].trim_start_matches(':');
                        let sender_nick = prefix.split('!').next().unwrap_or("onbekend");
                        let target_chan = parts[2];

                        // Parse de eigenlijke chattekst (alles na de dubbelepunt van arg 4)
                        if let Some(colon_idx) = raw[1..].find(':') {
                            let text = &raw[colon_idx + 2..];

                            if !sender_nick.eq_ignore_ascii_case(nick) {
                                let bridge_msg = BridgeMessage {
                                    source_platform: Platform::Irc,
                                    source_channel: target_chan.to_string(),
                                    author_name: sender_nick.to_string(),
                                    author_id: None,
                                    content: text.to_string(),
                                    reply_to: None,
                                    is_action: text.starts_with("\x01ACTION"),
                                };

                                let _ = self.inbound_tx.send(bridge_msg).await;
                            }
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
