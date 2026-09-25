# IRCord — Installation and Deployment Guide 🚀

This guide provides step-by-step instructions on how to install, configure, and operate the **IRCord** hybrid IRC-Discord bot daemon in production.

---

## 📋 Table of Contents
1. [System Requirements](#1-system-requirements)
2. [Configuring Discord Bot & Webhooks](#2-configuring-discord-bot--webhooks)
3. [Configuring IRC & SASL Account](#3-configuring-irc--sasl-account)
4. [Configuration Files (.env & config.toml)](#4-configuration-files-env--configtoml)
5. [Installation Option A: Docker Compose (Recommended)](#5-installation-option-a-docker-compose-recommended)
6. [Installation Option B: Bare-Metal / Native Rust](#6-installation-option-b-bare-metal--native-rust)
7. [Verification & Testing](#7-verification--testing)
8. [Troubleshooting & FAQ](#8-troubleshooting--faq)

---

## 1. System Requirements

### For the IRCord Daemon
Written in Rust, IRCord is exceptionally lightweight:
- **Memory:** < 20 MB RAM active runtime usage.
- **CPU:** 1 virtual core (0.25 - 0.5 vCPU is more than enough).
- **Disk Space:** ~50 MB for the binary, SQLite database, and logs.
- **Operating System:** Linux (Debian, Ubuntu, Alpine, Arch), macOS, or Windows.

### For FlashML FreeToken AI (Optional, if hosted locally)
- **RAM:** Minimum 8 GB RAM (16–32 GB recommended for 7B/14B MoE models).
- **GPU (Optional):** AMD ROCm (Radeon 780M/890M/RX 7000+) or NVIDIA CUDA. CPU co-execution is supported.
*Note: If you prefer not to host a local LLM, point the AI endpoint to any OpenAI-compatible server or leave it disabled.*

---

## 2. Configuring Discord Bot & Webhooks

To establish the bridge between IRC and Discord, a **Discord Bot Token** and **Discord Webhooks** are required.

### Step 2.1: Create Discord Application & Bot
1. Navigate to the [Discord Developer Portal](https://discord.com/developers/applications) and sign in.
2. Click **New Application** at top-right and assign a name (e.g. `IRCord`).
3. In the left navigation menu, click **Bot**.
4. Click **Reset Token** (or *Add Bot*) and copy the generated token. This is your `DISCORD_BOT_TOKEN`.
5. Scroll down to **Privileged Gateway Intents** and enable:
   - ✅ **MESSAGE CONTENT INTENT** *(Essential: without this intent, the bot cannot read channel messages)*
   - ✅ **SERVER MEMBERS INTENT** *(Recommended for presence tracking)*
6. Click **Save Changes**.

### Step 2.2: Invite Bot to Your Discord Server
1. In the left navigation menu, go to **OAuth2 ➔ URL Generator**.
2. Under **Scopes**, select: `bot`.
3. Under **Bot Permissions**, select:
   - `Send Messages`
   - `Manage Webhooks`
   - `Read Message History`
   - `Embed Links`
   - `Attach Files`
4. Copy the generated URL at the bottom and open it in your browser to authorize and add the bot to your Discord server.

### Step 2.3: Create Discord Webhooks
For each Discord channel linked to IRC, the bot uses a webhook to mirror IRC messages with the nickname and avatar of the IRC user:
1. Open Discord, right-click on the desired text channel, and select **Edit Channel** (gear icon).
2. Go to **Integrations ➔ Webhooks ➔ New Webhook**.
3. Name the webhook (e.g. `IRCord Relay`).
4. Click **Copy Webhook URL**. Save this URL for your `config.toml` mapping.

---

## 3. Configuring IRC & SASL Account

Many modern IRC networks (such as Libera.Chat, OFTC, or Ergo) require SASL authentication to authenticate directly during the TLS handshake before channel joins. This prevents 'nick in use' conflicts and grants access to registered-only (`+r`) channels.

1. Register your bot's nick with NickServ on your IRC network:
   ```text
   /msg NickServ REGISTER <password> <email-address>
   ```
2. Note down the account username and password. These correspond to `IRC_SASL_USER` and `IRC_SASL_PASS`.

---

## 4. Configuration Files (.env & config.toml)

### Step 4.1: Configure `.env`
Copy the example environment template:
```bash
cp .env.example .env
```

Open `.env` in your text editor and fill in your values:
```dotenv
# Discord Credentials
DISCORD_BOT_TOKEN=MTE5OT...your_actual_token_here...
GITHUB_WEBHOOK_SECRET=your_github_webhook_secret

# IRC Connection Settings
IRC_SERVER=irc.libera.chat
IRC_PORT=6697
IRC_NICK=IRCordBot
IRC_USER=ircord
IRC_REALNAME=IRCord Hybrid AI Daemon
IRC_PASSWORD=

# IRCv3 SASL Authentication
IRC_SASL_USER=IRCordBot
IRC_SASL_PASS=your_secret_sasl_password

# SQLite Database
DATABASE_URL=sqlite:ircord.db

# FlashML FreeToken Local Endpoint & Model
FREETOKEN_BASE_URL=http://127.0.0.1:1919/v1
FREETOKEN_MODEL=default
FREETOKEN_API_KEY=
```

### Step 4.2: Configure `config.toml`
Open `config.toml` and configure your general settings, channel pairings, and language:
```toml
[general]
language = "en" # "en" (English) or "nl" (Dutch) for chat output labels
bot_owner_discord_id = 123456789012345678
bot_owner_irc_nick = "Kuuke"
http_port = 9090
pastebin_threshold_lines = 4
admin_channel_irc = "#bot-logs"
admin_channel_discord_id = 0

[bridge]
loop_prevent_timeout_sec = 10
lru_cache_capacity = 2000
sync_presence = true
sync_edits = true

# Pair your channels here
[[channels]]
irc_channel = "#general"
discord_channel_id = 123456789012345678
discord_webhook_url = "https://discord.com/api/webhooks/..."

[whatpulse]
team_name = "Team de Apen"
api_url = "https://whatpulse.org/api/v1"
poll_interval_seconds = 180
cache_ttl_seconds = 300

[ai]
base_url = "http://127.0.0.1:1919/v1"
default_model = "default"
max_tokens = 300
temperature = 0.7
sliding_window_size = 8
hourly_token_budget = 50000

[moderation]
irc_flood_delay_ms = 800
irc_line_max_bytes = 380
raid_threshold_joins_per_sec = 5
raid_mute_duration_sec = 60
```

---

## 5. Installation Option A: Docker Compose (Dedicated File per Setup) 🐳

IRCord provides **3 dedicated, clean Docker Compose files** plus an interactive launcher so you can run exactly the version you want without any complicated flags:

### 0. Quick Interactive Launcher (Easiest)
- **Windows**: Double-click or run [`start.bat`](file:///d:/IRCord/start.bat)
- **Linux / NAS**: Run `./start.sh`

---

### 1. Ollama ROCm + GPU Setup (Recommended for Minisforum N5 Pro & Radeon 890M)
*Best for Minisforum N5 Pro, AMD Ryzen AI / Radeon iGPU, or systems wanting an all-in-one setup.*
```bash
docker compose -f docker-compose.ollama.yml up -d --build
# or simply 'docker compose up -d --build'
```
* **Ollama (ROCm GPU)** starts automatically.
* **`ollama-init`** automatically downloads the configured `FREETOKEN_MODEL` (e.g. `qwen2.5:7b`).
* **`ircord`** boots and connects directly to Ollama.

### 2. FlashML FreeToken Engine Setup
*For running the custom FlashML FreeToken ROCm 6.1 container.*
```bash
docker compose -f docker-compose.freetoken.yml up -d --build
```

### 3. Standalone IRCord Bot (External AI on Host/LAN)
*When you already run Ollama, vLLM, or LM Studio directly on your host OS or a separate server.*
```bash
docker compose -f docker-compose.standalone.yml up -d --build
```

### Step 5.2: Verify Status & Automatic Model Pull
```bash
# Check running containers
docker compose ps

# Follow IRCord daemon logs
docker compose logs -f ircord

# Check model installation progress (if using Ollama profile)
docker compose logs -f ircord-ollama-init
```

### Step 5.3: Manage Containers
- Stop stack: `docker compose down`
- Restart daemon: `docker compose restart ircord`
- Switch model: change `FREETOKEN_MODEL` in `.env` and restart.

---

## 6. Installation Option B: Bare-Metal / Native Rust

If you prefer to run IRCord directly on your host machine or VPS:

### Step 6.1: Prerequisites
Install the Rust toolchain (Rust 1.75+ recommended):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

Install SQLite libraries:
```bash
# Debian / Ubuntu
sudo apt-get update && sudo apt-get install -y libsqlite3-dev pkg-config libssl-dev build-essential
```

### Step 6.2: Build the Release Binary
```bash
cargo build --release
```
The optimized binary will be created at `target/release/ircord`.

### Step 6.3: Systemd Service (Linux Daemon)
Create a systemd unit file `/etc/systemd/system/ircord.service`:
```ini
[Unit]
Description=IRCord Hybrid IRC-Discord AI Daemon
After=network.target

[Service]
Type=simple
User=ircord
WorkingDirectory=/opt/ircord
ExecStart=/opt/ircord/target/release/ircord
Restart=always
RestartSec=5
EnvironmentFile=/opt/ircord/.env

[Install]
WantedBy=multi-user.target
```

Enable and start the service:
```bash
sudo systemctl daemon-reload
sudo systemctl enable ircord
sudo systemctl start ircord
sudo journalctl -u ircord -f
```

---

## 7. Verification & Testing

1. **Check Web API Health:**
   ```bash
   curl http://localhost:9090/health
   # Expected response: {"status":"healthy"}
   ```

2. **Check Diagnostics Buffer:**
   ```bash
   curl http://localhost:9090/api/errors
   ```

3. **In-Chat Smoke Test:**
   - On IRC or Discord: Type `!ping` (bot responds `pong!`).
   - Type `!status` (bot reports uptime, active plugins, and memory).
   - Type `!weather Amsterdam` or `!weer Amsterdam`.
   - Type `!crypto btc`.
   - Post a YouTube link to verify metadata and description parsing.

---

## 8. Troubleshooting & FAQ

### Issue: Bot joins IRC but does not receive Discord messages
- **Cause:** Privileged Gateway Intent missing in Discord Developer Portal.
- **Solution:** Go to Discord Developer Portal ➔ Application ➔ Bot ➔ Enable **MESSAGE CONTENT INTENT**.

### Issue: Bot cannot join `+r` channels on IRC
- **Cause:** SASL credentials are missing or incorrect.
- **Solution:** Verify `IRC_SASL_USER` and `IRC_SASL_PASS` in `.env`. Ensure your nick is registered with NickServ.

### Issue: Logs or errors flooding public channels
- **Design:** Full diagnostics (`!errors`) are restricted to PM or the designated admin log channel (`#bot-logs` on IRC, configurable Discord channel ID). Public channels receive a brief redirect notice.
