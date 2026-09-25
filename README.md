# IRCord 🤖⚡

> **High-Performance Hybrid IRC-Discord AI Bot Daemon in Rust**

[![Rust](https://img.shields.io/badge/rust-2021_edition-orange.svg)](https://www.rust-lang.org/)
[![Tokio](https://img.shields.io/badge/async-tokio_1.40-blue.svg)](https://tokio.rs/)
[![Serenity](https://img.shields.io/badge/discord-serenity_0.12-5865F2.svg)](https://github.com/serenity-rs/serenity)
[![SQLite](https://img.shields.io/badge/database-sqlite_fts5-003B57.svg)](https://www.sqlite.org/)
[![FlashML](https://img.shields.io/badge/ai-FlashML_FreeToken-green.svg)](https://github.com/FlashML-org/FreeToken)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-lightgrey.svg)]()

> 🇳🇱 **[Bekijk deze documentatie in het Nederlands (README.nl.md)](README.nl.md)**

---

## 📖 Table of Contents
- [Overview](#-overview)
- [Key Features](#-key-features)
- [Architecture](#-architecture)
- [Available Plugins & Commands](#-available-plugins--commands)
- [Edge-Native AI & RAG](#-edge-native-ai--rag)
- [Quick Start](#-quick-start)
- [Configuration](#-configuration)
- [Project Structure](#-project-structure)
- [Installation & Deployment](#-installation--deployment)
- [License](#-license)

---

## 🚀 Overview

**IRCord** is a modular, type-safe, and resource-efficient daemon written in **Rust**. It bridges classic **IRC networks** (Internet Relay Chat) and modern **Discord servers** in real time.

Drawing inspiration from classic bots such as [CloudBot](https://github.com/TotallyNotRobots/CloudBot) and [IRCPlus](https://github.com/Cjefke/IRCPlus), IRCord pairs rich community features with the power of **local edge AI** through [FlashML FreeToken](https://github.com/FlashML-org/FreeToken).

### Why IRCord?
- 🦀 **Pure Rust:** Maximum reliability, zero garbage-collection pauses, and guaranteed thread safety.
- 🪶 **Minimal Footprint:** Active runtime memory footprint typically under **20 MB RAM**.
- 🛡️ **Panic-Proof Plugins:** Plugins execute within isolated `catch_unwind` boundaries; a failure in one plugin never crashes the daemon.
- 🔒 **Privacy-First AI:** Local LLM inference (such as DeepSeek, Qwen, or Llama) via an OpenAI-compatible endpoint; no sensitive chat logs sent to third-party clouds.
- 🌐 **Multilingual Aliases (i18n):** Native English canonical commands paired with locale aliases (Dutch, German, etc.).

---

## ✨ Key Features

1. **Bidirectional Bridge (IRC ⇄ Discord)**
   - Real-time message synchronization with sub-millisecond dispatching.
   - Discord Webhook dispatching with dynamic nicknames and avatars matching IRC users.
   - Discord native replies formatted cleanly for IRC: `<(Discord) Alice ↳ Bob>: Absolutely, that works!`
   - Anti-ping protection (zero-width spaces inserted into usernames) to prevent unwanted mentions.
   - Bidirectional LRU deduplication cache to eliminate relay loops.

2. **Multi-Channel & Multi-Server Matrix**
   - Pair an arbitrary number of IRC channels with Discord channels in a single daemon instance.
   - Dynamic configuration validation (`config.toml`).

3. **Modular Plugin System**
   - 25 built-in native plugins covering community tools, moderation, statistics, media, gaming, and AI.
   - Support for hot-reloadable **[Rhai](https://rhai.rs/) scripts** in `./scripts/` for dynamic custom commands without recompiling.

4. **Local Edge AI & Vision**
   - Direct integration with **FlashML FreeToken** (OpenAI-compatible `/v1/chat/completions` API).
   - RAG (Retrieval-Augmented Generation) powered by **SQLite FTS5** full-text search index.
   - Automated AI Vision Alt-Text generation for Discord images relayed to IRC.

5. **Community Statistics: WhatPulse Integration**
   - Real-time keystroke, mouse click, and team rankings.
   - Supports both the official WhatPulse API and custom REST endpoints (e.g. `grandmasg.nl`).
   - Profile nickname linking (`!wp link <username>`).

6. **Security & Channel Moderation**
   - IRCv3 SASL authentication (secure authentication before joining `+r` channels).
   - Flood guard with configurable delays and line byte limits.
   - Anti-raid and clone join detection.
   - Automatic pastebin threshold for long code snippets (> 4 lines).
   - Live token and secret leak detection via `SafetyPlugin`.

7. **Observability & Webhooks**
   - Built-in Axum HTTP server on port `9090`.
   - `/health` endpoint for Docker and Kubernetes health probes.
   - `/metrics` endpoint for uptime, memory, and cache statistics.
   - `/api/errors` endpoint for real-time diagnostics buffer.
   - `/api/github` webhook endpoint with HMAC SHA-256 signature verification.

---

## 🏛️ Architecture

```text
┌────────────────┐      TLS Socket (Async)     ┌────────────────────────────────────────────────────────┐
│   IRC Server   │ ◄─────────────────────────► │                  Rust Daemon (Tokio)                   │
│ (Libera/Local) │                             │                                                        │
└────────────────┘                             │  ┌──────────────┐         ┌─────────────────────────┐  │
                                               │  │ irc_client   │──mpsc──►│   Bridge Router (LRU)   │  │
┌────────────────┐     Discord Gateway /       │  └──────────────┘         └────────────┬────────────┘  │
│  Discord Guild │ ◄─ Webhooks (Avatars/Nicks) │  ┌──────────────┐                      │               │
└────────────────┘                             │  │ discord_task │◄──────mpsc───────────┤               │
                                               │  └──────────────┘                      ▼               │
┌────────────────┐     HTTP Webhooks           │  ┌──────────────┐         ┌─────────────────────────┐  │
│ GitHub / Feeds │──(HMAC Verified)───────────►│  │ axum_http_srv│──mpsc──►│  Plugin Manager (25)    │  │
└────────────────┘                             │  └──────────────┘         └────────────┬────────────┘  │
                                               │                                        │               │
                                               │        ┌──────────────┬────────────────┼─────────────┐ │
                                               │        ▼              ▼                ▼             ▼ │
                                               │   [FreeToken AI]  [WhatPulse]     [Presence/AFK]   [..]│
                                               └────────┼──────────────┼────────────────────────────────┘
                                                        │              │ HTTP REST (Cached)
                                                        │              ▼
                                                        │    ┌──────────────────────────────────────────┐
                                                        │    │  WhatPulse API ("Team de Apen")          │
                                                        │    └──────────────────────────────────────────┘
                                                        ▼
                                               ┌────────────────────────────────────────────────────────┐
                                               │                FlashML FreeToken Engine                │
                                               │        (Local Edge MoE: DeepSeek / Qwen / etc.)        │
                                               └────────────────────────────────────────────────────────┘
```

---

## 🧩 Available Plugins & Commands

All commands accept either an exclamation mark (`!`) or a period (`.`):

| Plugin | Canonical & Alias Triggers | Description | Example |
|---|---|---|---|
| **WhatPulse** | `!wp`, `!whatpulse` | Team or user keystrokes and click stats. Link nick with `!wp link`. | `!wp`, `!wp Alice`, `!wp link Alice` |
| **AI Suite** | `!ai` | Ask a question to the local FreeToken LLM with channel context. | `!ai Explain monads simply.` |
| | `!ai models` | Displays the active model and all available models on the local AI server. | `!ai models` |
| | `!ai model <name>` | **(Admin/Operator only)** Dynamically switch the active AI model without restart. | `!ai model qwen2.5-coder` |
| | `!tldr [url]` | Summarizes recent chat logs or extracts and summarizes a web article in 2 bullets. | `!tldr https://news.ycombinator.com` |
| | `!catchup [count]` | Personal absence briefing summarizing recent discussions, commitments, and mentions. | `!catchup`, `!catchup 50` |
| | `!vibe`, `!sentiment` | Evaluates channel sentiment, mood percentage, and trending topics in 1-2 lines. | `!vibe` |
| | `!roast <nick>` | Generates a witty, playful nerd roast in classic IRC style. | `!roast Bob` |
| | `!whatis <term>`, `!def` | Razor-sharp, factual 1-line definition for technical terms or acronyms. | `!whatis BGP`, `!whatis Docker` |
| | `!topic suggest` | AI generates a creative, relevant channel topic suggestion. | `!topic suggest` |
| **Translate** | `!translate`, `!tr`, `!vertaal` | Translates text using local AI (with seamless fallback to web translation). | `!tr en:de Good morning!`, `!tr Where is the train?` |
| **Weather** | `!weather`, `!weer`, `!wetter`| Current weather, temperature, and wind speed via Open-Meteo. | `!weather Amsterdam`, `!weather Tokyo` |
| **Time** | `!time`, `!tijd`, `!clock`, `!klok` | Real-time world clock and timezone information for any city or country. | `!time Tokyo`, `!time New York` |
| **Crypto & FX**| `!crypto`, `!coin` | Live cryptocurrency prices in EUR & USD with 24h trends (CoinGecko). | `!crypto btc`, `!crypto eth`, `!crypto sol` |
| | `!currency`, `!valuta`, `!fx` | Real-time fiat exchange rates via European Central Bank (ECB/Frankfurter). | `!currency 100 usd eur`, `!valuta 50 gbp to eur` |
| **Wikipedia** | `!wiki`, `!wkp` | Concise article summaries from Wikipedia (supports language selection). | `!wiki Linux`, `!wiki en Alan Turing` |
| **Urban Dict** | `!ud`, `!urban`, `!slang` | Slang definitions and examples from Urban Dictionary. | `!ud yeet`, `!ud poggers` |
| **Minecraft** | `!mc`, `!minecraft` | Pings a Minecraft Java server for online status, player count, and MOTD. | `!mc play.hypixel.net` |
| **Tell (Memos)**| `!tell`, `!memo`, `!note` | Leave offline memos; automatically delivered when recipient speaks. | `!tell Bob Please review the latest pull request` |
| **YouTube** | `!yt`, `!youtube` | Search videos or inspect links via oEmbed (title, uploader, description). | `!yt lofi hip hop beats` |
| **Google Search** | `!g`, `!google`, `!search` | Web search with title, concise snippet, and link (DuckDuckGo / Google CSE). | `!g rust lang documentation` |
| **Presence** | `!seen`, `!lastonline` | Shows when a user was last active and their last action. | `!seen Alice` |
| | `!online` | Overview of active users across IRC and Discord. | `!online` |
| **AFK** | `!afk` | Toggles AFK status; automatically replies when mentioned. | `!afk Grabbing coffee` |
| **Slap** | `!slap`, `!mep` | Classic IRC trout slap with platform-appropriate formatting. | `!slap Bot` |
| **Quotes** | `!quote`, `!q` | Save and retrieve memorable channel quotes. | `!quote add <text>`, `!quote random` |
| **Karma** | `!karma`, `++`, `--` | Tracks karma scores for subjects and nicknames. | `rust++`, `bugs--`, `!karma rust` |
| **Poll** | `!poll` | Interactive multi-choice channel poll. | `!poll Pizza tonight? \| Yes \| No` |
| **Remind** | `!remind`, `!remindme` | Sets a timer reminder. | `!remind 10m Check server backup!` |
| **Alias** | `!alias` | Custom channel alias management. | `!alias add docs https://rust-lang.org` |
| **Birthdays** | `!bday`, `!verjaardag` | Register birthdays (`!bday set DD-MM[-YYYY]`), view upcoming birthdays (`!bday next`), or check a friend's date (`!bday nick`). The bot automatically sends morning greetings with age and cake! | `!bday set 24-09`, `!bday next`, `!bday Alice` |
| **Admin & Logs**| `!status`, `!ping`, `!stats` | Reports uptime, active plugins, memory, and database metrics. | `!status` |
| | `!errors`, `!errorlog` | Shows recent errors or panics in PM or `#bot-logs`. Use `clear` to reset. | `!errors 5`, `!errors clear` |
| **Identity & Bridge**| `!link`, `!whois` | Link IRC nick and Discord account via 6-digit OTP code. View profiles, linked identities, karma, and birthdays. | `!link @Alice`, `!link verify 123456`, `!whois Alice` |
| | `!bridge stats`, `!top` | Shows total bridge metrics (message count, IRC vs Discord distribution) and top chatters. | `!bridge stats` |
| **Sysadmin & NAS** | `!nas`, `!hw`, `!sysinfo` | Telemetry for Minisforum N5 Pro NAS: OS, CPU load, RAM usage, and Radeon 890M GPU status. | `!nas` |
| | `!dns <domain> [type]` | Fast DNS resolution via trustless DNS-over-HTTPS (DoH). Supports A, AAAA, MX, TXT, CNAME. | `!dns tweakers.net A` |
| | `!ssl <domain>`, `!http <url>`| Validates HTTPS/TLS handshake and HSTS, or probes HTTP latency, status code, and headers (SSRF-protected). | `!ssl tweakers.net`, `!http https://site.com` |
| **RSS Feeds** | `!rss add/list/del/latest`| Subscribes to RSS/Atom feeds. Background task polls feeds every 10 minutes and broadcasts new articles. | `!rss add https://tweakers.net/feeds/nieuws.xml #general` |
| **Tech & GitHub** | `!gh`, `!github <repo>` | Fetches stars, open issues, description, and latest release from the official GitHub REST API. | `!gh rust-lang/rust` |
| | `!cve`, `!security <id>` | Looks up vulnerability details and CVSS security scores from the official OSV.dev / NIST database. | `!cve CVE-2024-3094` |
| **Sed (Passive)** | `s/old/new/` | Automatically corrects typos from your previous message. | `s/teh/the/` |
| **URL Titler** | *Automatic* | Detects URLs in chat and posts page `<title>` previews. | `https://github.com/...` |
| **Safety** | *Automatic* | Detects and warns when someone accidentally leaks API keys or tokens. | — |
| **Reactions** | *Automatic* | Responds to specific community keywords and greetings. | — |

---

## 🧠 Edge-Native AI & RAG

IRCord eliminates the need for expensive third-party cloud tokens. It is built to communicate with a local instance of **FlashML FreeToken**:
- **OpenAI Compatible:** Standard `/v1/chat/completions` API.
- **RAG via SQLite FTS5:** Full-text search over indexed historical chat logs for context-aware answers.
- **Vision Alt-Text:** Discord images relayed to IRC receive an automated 1-line description.
- **Token Budget Guard:** Configurable hourly token limit (`hourly_token_budget`) preventing infinite loops.

---

## ⚡ Quick Start

### 1. Clone the repository
```bash
git clone https://github.com/your-org/ircord.git
cd ircord
```

### 2. Configure environment and settings
```bash
cp .env.example .env
cp config.toml config.toml
```

Edit `.env` with your credentials:
- `DISCORD_BOT_TOKEN`: Discord Bot Token.
- `IRC_SERVER` & `IRC_NICK`: Desired IRC network and nickname.

Map your channels in `config.toml`:
```toml
[[channels]]
irc_channel = "#mychannel"
discord_channel_id = 123456789012345678
discord_webhook_url = "https://discord.com/api/webhooks/..."
```

### 3. Launch with Docker Compose (Server of Choice) 🐳
Choose your desired AI backend in `.env`:
- `COMPOSE_PROFILES=ollama` (Recommended: installs Ollama with GPU acceleration and automatically pulls your model, e.g. `qwen2.5:7b`).
- `COMPOSE_PROFILES=freetoken` (Builds and runs FlashML FreeToken).
- `COMPOSE_PROFILES=standalone` (Runs only the IRCord bot for an external AI server on host/LAN).

Launch with a single command:
```bash
docker compose up -d --build
```

View live logs and model download progress:
```bash
docker compose ps
docker compose logs -f ircord
docker compose logs -f ircord-ollama-init
```

---

## ⚙️ Configuration

`config.toml` manages all daemon runtime parameters:

```toml
[general]
language = "en" # "en" (English) or "nl" (Dutch) for chat output labels
bot_owner_discord_id = 0
bot_owner_irc_nick = "Kuuke"
http_port = 9090
pastebin_threshold_lines = 4
admin_channel_irc = "#bot-logs"
admin_channel_discord_id = 0 # Optional: Discord channel ID for #bot-logs

[bridge]
loop_prevent_timeout_sec = 10
lru_cache_capacity = 2000
sync_presence = true
sync_edits = true

[[channels]]
irc_channel = "#general"
discord_channel_id = 123456789012345678
discord_webhook_url = "https://discord.com/api/webhooks/..."

[whatpulse]
team_name = "Team de Apen"
api_url = "https://whatpulse.org/api/v1" # See https://whatpulse.org/help/api/web/intro
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

## 📂 Project Structure

```text
IRCord/
├── migrations/                # SQLite database migrations (schema & FTS5)
│   └── 20260908_init.sql
├── scripts/                   # Hot-reloadable Rhai scripts for community commands
│   └── hello.rhai
├── src/
│   ├── ai/                    # FreeToken client, RAG searcher & Vision helpers
│   ├── bridge/                # Bridge router, normalizer & deduplication cache
│   ├── config.rs              # TOML configuration parser & validator
│   ├── discord/               # Serenity Gateway handler & Webhook dispatcher
│   ├── irc/                   # IRC client task, SASL handler & flood guard
│   ├── plugins/               # 25 modular plugins (WhatPulse, AI, Moderation, etc.)
│   ├── utils/                 # Lifecycle, error logger & formatting helpers
│   ├── web/                   # Axum HTTP server (/health, /metrics, /api/errors)
│   └── main.rs                # Daemon startup sequence & dispatch loops
├── docs/
│   ├── DESIGN_SPEC.md         # Original architectural specification and design plan (English)
│   └── DESIGN_SPEC.nl.md      # Original architectural specification and design plan (Dutch)
├── .env.example               # Example environment variables (English)
├── .env.nl.example            # Example environment variables (Dutch)
├── Cargo.toml                 # Rust dependencies & package metadata
├── config.toml                # Application configuration
├── Dockerfile                 # Multi-stage Alpine container for minimal footprint
├── Dockerfile.freetoken       # FlashML FreeToken AI inference container
├── docker-compose.yml         # Full stack orchestration
├── INSTALL.md                 # Installation and deployment guide (English)
├── README.nl.md               # Complete Dutch documentation
└── README.md                  # Main documentation (English)
```

---

## 📚 Installation & Deployment

For a comprehensive guide covering Discord Developer Portal setup, SASL account configuration, bare-metal installation, and systemd services, refer to:

👉 **[INSTALL.md](INSTALL.md)**

---

## 📄 License

Distributed under the **MIT** or **Apache 2.0** license. See source files for details.
