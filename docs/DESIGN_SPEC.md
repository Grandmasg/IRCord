# Project & Architectural Specification: Hybrid IRC-Discord AI Bot (Rust Edition)

> [!NOTE]
> **Archived Design Specification & RFC**  
> This document contains the original architectural blueprint and design specification for **IRCord**.  
> For up-to-date documentation, commands, and installation guides, please refer to:
> - [README.md](../README.md) (Official primary documentation)
> - [README.nl.md](../README.nl.md) (Dutch user & operator guide)
> - [INSTALL.md](../INSTALL.md) (Installation & deployment guide)

---

## 1. Vision & Objectives
The goal of IRCord is to deliver a **high-performance, modular, and type-safe bot daemon** written in **Rust**. The bot bridges classic Internet Relay Chat (IRC) channels with modern Discord guilds, combining the rich culture and utilities of classic IRC bots (such as [Cjefke/IRCPlus](https://github.com/Cjefke/IRCPlus) and [TotallyNotRobots/CloudBot](https://github.com/TotallyNotRobots/CloudBot)) with local edge AI powered by **[FlashML FreeToken](https://github.com/FlashML-org/FreeToken)**.

Key core capabilities:
1. **Bidirectional Relay (Bridge)**: Seamless message flow between IRC and Discord with dynamic webhook avatars, anti-ping zero-width spaces, Discord Reply-to-IRC context formatting, and cross-platform mention resolution.
2. **Multi-Channel & Multi-Server Support**: A single daemon synchronizing multiple channel pairs simultaneously via validated configuration with live reload support.
3. **Modular Plugin Architecture (CloudBot & IRCPlus Philosophy)**: Extensible plugin design using an asynchronous Rust Trait combined with sandboxed hot-reloadable scripting via [Rhai](https://rhai.rs/) for community commands without recompilation.
4. **Local Edge AI**: Integration with local LLMs via FlashML FreeToken (`/v1/chat/completions`) for strict privacy, zero cloud costs, and fast inference on consumer hardware.
5. **AI Vision & Image Alt-Text**: Concise automatic descriptions of Discord images and screenshots for IRC users (with hash-based result caching).
6. **Smart AI Utilities**: `!tldr` (conversation summarization), `!topic suggest`, translation, and SQLite FTS5 RAG retrieval over channel history.
7. **Presence, AFK & Last Online Hub**: Hybrid presence detection (`!online`, `!lastonline`, `!seen`, quit reasons, and AFK notifications).
8. **WhatPulse Community Statistics ("Team de Apen")**: Real-time team metrics (keystrokes, mouse clicks, ranking, and personal milestones) via the official WhatPulse Web API v1 and custom endpoints.
9. **Personal & Public Feeds (RSS/Atom & Keyword Alerts)**: Channel newsfeeds alongside private subscriptions via DM/Query with keyword triggers and AI digests.
10. **Security & Abuse Hardening**: Role-Based Access Control (Ops/Mods/Owner), anti-raid/clone protection, IRC flood control, auto-pastebin for multi-line snippets (>4 lines), NickServ verification, and GitHub HMAC signature validation.
11. **Enterprise Observability & Graceful Lifecycle**: Structured tracing spans, controlled zero-data-loss shutdown flow, and predictable memory usage (< 20 MB RAM).

---

## 2. System & Plugin Architecture

At the heart of the bot is a centralized event router that normalizes incoming events from both IRC and Discord and dispatches them to the **Plugin Manager**:

```text
┌────────────────┐      TLS Socket (Async)     ┌────────────────────────────────────────────────────────┐
│   IRC Server   │ ◄─────────────────────────► │                  Rust Daemon (Tokio)                   │
│ (Libera/Custom)│                             │                                                        │
└────────────────┘                             │  ┌──────────────┐         ┌─────────────────────────┐  │
                                               │  │ irc_task     │──mpsc──►│   Multi-Channel Router  │  │
┌────────────────┐     Discord Gateway /       │  └──────────────┘         └────────────┬────────────┘  │
│  Discord Guild │ ◄─ Webhooks (Avatars/Nicks) │  ┌──────────────┐                      │               │
└────────────────┘                             │  │ discord_task │◄──────mpsc───────────┤               │
                                               │  └──────────────┘                      ▼               │
┌────────────────┐     HTTP Webhooks           │  ┌──────────────┐         ┌─────────────────────────┐  │
│ GitHub / Feeds │──(HMAC Verified)───────────►│  │ axum_http_srv│──mpsc──►│  Plugin Manager (RBAC)  │  │
└────────────────┘                             │  └──────────────┘         └────────────┬────────────┘  │
                                               │                                        │               │
                                               │        ┌──────────────┬────────────────┼─────────────┐ │
                                               │        ▼              ▼                ▼             ▼ │
                                               │   [FreeToken AI]  [WhatPulse]     [Presence/AFK]   [..]│
                                               └────────┼──────────────┼────────────────────────────────┘
                                                        │              │ HTTP REST (Cached)
                                                        │              ▼
                                                        │    ┌──────────────────────────────────────────┐
                                                        │    │  https://whatpulse.org/api/v1/           │
                                                        │    │  (Official Web API v1 - Team de Apen)    │
                                                        │    └──────────────────────────────────────────┘
                                                        ▼
                                               ┌────────────────────────────────────────────────────────┐
                                               │                FlashML FreeToken Engine                │
                                               │        (Local Edge MoE: DeepSeek / Qwen / etc.)        │
                                               └────────────────────────────────────────────────────────┘
```

---

## 3. Advanced Bridge UX, Multi-Channel & Formatting

### 3.1 Multi-Channel Mapping & Validated Configuration with Live Reload
A single daemon manages multiple channels concurrently via matrix mappings in `config.toml`.
* **Schema Validation:** Serde parses and validates:
  * No duplicate IRC channels or Discord channel IDs.
  * Correct format for Discord webhook URLs (`https://discord.com/api/webhooks/...`).
* **Live Reload:**
  * Background file-watching or Unix signals reload `config.toml` dynamically.
  * Channel mappings and keywords update on-the-fly without resetting TLS sockets or Discord gateway sessions.

### 3.2 Discord Replies to IRC Context
When a Discord user responds using Discord's native "Reply" feature:
* Formatted cleanly on IRC:  
  `<(Discord) Alice ↳ Bob>: Exactly right!`  
  This preserves full conversational context on IRC even in busy channels.

### 3.3 Automatic Pastebin for Long Code Blocks
* Output or messages exceeding 4 lines are automatically uploaded to a lightweight pastebin (e.g., `0x0.st`), outputting a neat summary to IRC:  
  `* [Codeblock: 32 lines] https://0x0.st/xyz.txt`

### 3.4 Discord Mentions & mIRC Color Sanitization
* `<@123456789>` resolves to `@Username`.
* `<#987654321>` resolves to `#channel-name`.
* Custom emojis `<:name:12345>` become `:name:`.
* mIRC color codes (`\x03`, `\x02`, `\x1f`) are stripped prior to Discord webhook transmission.
* Zero-width space (`\u{200B}`) is inserted into author nicks on IRC to prevent unintended highlight pings.

### 3.5 Bridge Consistency, Loop Prevention & Message ID Mapping
To completely prevent infinite feedback loops (`IRC -> Discord -> IRC`) and maintain reply fidelity:
* **Loop-Guard Ingress Filter:**
  * Discord webhook messages carry a `webhook_id` and are immediately dropped by the gateway listener.
  * Messages from the bot's own ID are discarded on both networks.
* **Message ID Cache (Ringbuffer / LRU Cache):**
  * The last 2,000 relayed messages are indexed in an in-memory LRU cache (`Discord Message ID <-> IRC Nick + Timestamp`).
  * When a Discord user replies to a bridged message, the bot matches the target in cache to extract the original author (`Alice ↳ Bob`).
* **Optional Message Edit Sync:**
  * If a Discord user edits a message within 60 seconds, a subtle correction is forwarded to IRC:  
    `* [Edit] Alice: <updated text>`.

### 3.6 Discord Rate Limiting & Webhook Retry Strategy
* **Rate-Limit Buckets:** Handled efficiently via token buckets per route.
* **Webhook Retry Backoff:**
  * Outgoing webhook requests are buffered in an MPSC queue.
  * On `HTTP 429 Too Many Requests`, the worker respects the `retry_after` header and backs off exponentially (max 3 retries) without blocking the IRC loop.

### 3.7 Plugin Lifecycle & Fault Tolerance
* **Panic Boundaries (`catch_unwind`):**
  * Every plugin invocation (`on_command`, `on_message`) runs within `tokio::task::spawn` and `std::panic::AssertUnwindSafe`.
  * If a plugin panics (e.g., unexpected regex mismatch or index error), **only that specific task fails**, while the core bridge and other plugins continue running uninterrupted.
* **Health & Restart:**
  * Errors are logged with structured tracing (`tracing::error!`) and recorded in the thread-safe `ErrorLogger` for `!errors` inspection.

### 3.8 Graceful Shutdown Flow (Zero Data Loss)
Upon receiving `SIGINT` (Ctrl+C), `SIGTERM` (Docker stop), or an owner `!shutdown` command:
1. **Broadcast Cancellation:** A `CancellationToken` instructs background tasks to reject new events.
2. **Queue Draining:** Outgoing webhook and pastebin queues are given 5 seconds to drain remaining messages.
3. **IRC Clean Disconnect:** Sends an official `QUIT :IRCord daemon gracefully shutting down...` to IRC and terminates TLS.
4. **Discord Gateway Disconnect:** Closes the Serenity shard cleanly.
5. **Database WAL Checkpoint:** Closes connection pools and enforces an SQLite WAL checkpoint (`pool.close().await`) ensuring zero data corruption.

---

## 4. Role-Based Access Control (RBAC) & Security Hardening

### 4.1 RBAC Levels
* **Level 0 (Public):** Standard utility commands (`!wp`, `!weather`, `!calc`, `!seen`, `!online`, personal DM feeds, `!ai` with standard cooldown).
* **Level 1 (Trusted / Voiced `+v` / Active Members):** Reduced AI cooldown, access to `!tldr` and `!quote add`.
* **Level 2 (Channel Ops `@` / Discord Moderators):** `!topic`, `!kick`, `!silence`, `!paste`, public channel feed management (`!rss add`).
* **Level 3 (Bot Owner):** `!reload scripts`, `!setmodel`, `!metrics`, `!errors`, `!shutdown` (verified via NickServ account name on IRC or static Discord Owner ID).

### 4.2 Security & Abuse Hardening
* **IRC Nick Spoofing Prevention:** Level 2 & 3 commands verify NickServ account identification status (`STATUS <nick>` / `ACC <nick>`).
* **SQL Injection Safety:** All queries strictly use `sqlx` parameterized macros (`query!`, `query_as!`).
* **Rhai Script Sandboxing:** Embedded Rhai engine restricts max operations (50,000), max string length (1,000 bytes), and forbids all filesystem and network access.
* **Anti-Impersonation:** Sanitizes Discord webhook display names to prevent IRC users from impersonating administrators.

---

## 5. WhatPulse Community Integration ("Team de Apen")

WhatPulse community statistics are deeply rooted in Dutch IRC history. The bot supports **two flexible sources**:
1. **Official WhatPulse Web API v1** (`https://whatpulse.org/api/v1/`, see [WhatPulse API Documentation](https://whatpulse.org/help/api/web/intro)) with Bearer token authentication via `WHATPULSE_API_KEY`.
2. **Local Client API or Custom Proxy Endpoints** (e.g. `http://localhost:3490/v1/account-totals` or custom aggregate feeds).

### 5.1 Commands
* **`!wp` / `!whatpulse`:** Shows live team statistics for **Team de Apen** (Keys, clicks, team rank, data uploaded/downloaded).
* **`!wp user [nick]` / `!wp <nick>` / `!wp me`:** Displays personal statistics for a linked profile or yourself.
* **`!wp link <username>`:** Links an IRC nick or Discord user to a WhatPulse username in SQLite.
* **`!wp top`:** Displays the top 5 typers and clickers in the team.
* **Live Pulse & Milestone Alerts:** Automatic notification whenever team members submit pulses or pass significant milestones.

---

## 6. Complete Plugin Catalog

### Category 1: AI Superpowers & Model Management (FlashML FreeToken)
* **`plugin_ai` (`!ai <prompt>` or `@bot mention`):** Direct queries to local edge LLM via `/v1/chat/completions`. Sliding memory window (last 8 interactions), automatic IRC line chunking (max 380 bytes, 800ms delay).
* **AI Model Management & Token Budget:** Dynamic switching via `!setmodel <name>` with hourly token caps to prevent overloading local hardware.
* **`plugin_vision` (AI Image Alt-Text for IRC):** Analyzes Discord images via local VLM and relays concise descriptions to IRC users with hash-based caching.
* **`plugin_tldr` (`!tldr [messages/minutes]`):** Generates structured 3-bullet conversation summaries from SQLite history.
* **`plugin_catchup` (`!catchup [count]` / `!digest`):** Personal absence summary highlighting discussions, commitments, and mentions.
* **`plugin_vibe` (`!vibe` / `!sentiment`):** Gauges real-time channel mood, trending topics, and sentiment percentage.
* **`plugin_topic` (`!topic suggest`):** Analyzes recent conversation to suggest fitting channel topics.
* **`plugin_translate` (`!tr <target_lang> <text>`):** Fast translation powered by DeepL v2 API, local FreeToken AI, or MyMemory fallback.
* **`plugin_rag_history` (`!ai who said what about X?`):** Searches SQLite FTS5 index to provide context-augmented answers.

### Category 2: Community Stats & WhatPulse
* **`plugin_whatpulse` (`!wp`, `!wp user`, `!wp link`, `!wp top`):** Live metrics from WhatPulse Web API v1.
* **`plugin_stats` (`!top`, `!peak`):** Channel activity leaderboards and all-time concurrent user peaks.

### Category 3: Presence, AFK & Last Online Hub
* **`plugin_last_online` (`!lastonline <nick>`, `!seen <nick>`):** Distinguishes between **last spoke** and **last active** (with quit reasons).
* **`plugin_online` (`!online`, `!users`):** Combined network roster showing IRC voices/ops and Discord online/voice activity.
* **`plugin_afk` (`!afk [reason]`):** Manages away status and alerts users mentioning AFK members.
* **`plugin_tell` (`!tell <nick> <message>`):** Offline memo delivery as soon as the recipient returns or chats.

### Category 4: Newsfeeds & Personal Alerts
* **Public Feeds (`!rss add <url> [channel]`, `!rss list`):** Auto-posts new articles to specified channels.
* **Private Feeds via DM (`!rss sub <url>`):** Tailored personal news delivered via private query.
* **Keyword Alerts (`!track <keyword>`):** Monitors feeds for specific keywords and sends instant DM notifications.
* **Personal AI News Digest (`!digest`):** Morning briefing summarized into 5 key points by local AI.

### Category 5: Classic IRC & Eggdrop Culture
* **`plugin_slap` (`!slap <target>`):** Traditional trout slapping (`\x01ACTION slaps <target> around a bit with a large trout\x01`).
* **`plugin_remind` (`!remindme <time> <message>`):** Asynchronous timers stored persistently in SQLite.
* **`plugin_quotes` (`!quote add <text>`, `!quote [search]`, `!quote random`):** Channel quote database (QDB).
* **`plugin_karma` (`nick++`, `nick--`, `!karma <nick>`):** Community reputation tracker with self-voting prevention.
* **`plugin_sed` (`s/old/new/` or `s/typo/fix/g`):** Fixes typos in previous messages.
* **`plugin_dice` (`!roll [XdY]`, `!choose <a/b/c>`):** Dice rolling and choice utilities.
* **`plugin_birthday` (`!bday set DD-MM[-YYYY]`, `!bday next`, `!bday [nick]`, `!bday del`):** Community birthday registry with automated morning congratulations and age calculation.

### Category 6: Community Utility, Interaction & Bridges
* **`plugin_alias` (`!alias add !name <response>`, `!alias list`):** Custom trigger shortcuts stored in SQLite.
* **`plugin_poll` (`!poll start "Question?" opt1/opt2`, `!poll vote <opt>`):** Cross-platform polls with Discord button support and IRC text voting.
* **`plugin_reactions`:** Relays Discord emoji reactions to IRC (`* Alice reacted with :thumbsup: on Bob's message`).
* **`plugin_file_upload` (`!upload <file>`, `!img <url>`):** Bridges links and files between platforms.

### Category 7: Information, Feeds & URL Safety
* **`plugin_youtube` (`!yt <search>`):** Official YouTube Data API v3 search with oEmbed metadata and description previews.
* **`plugin_google` (`!g <query>`):** Official Google Custom Search JSON API with DuckDuckGo fallback.
* **`plugin_wiki` (`!wiki <query>`):** Wikimedia REST API summaries (Dutch with English fallback).
* **`plugin_urban` (`!ud <slang>`):** Urban Dictionary definitions and sample sentences.
* **`plugin_minecraft` (`!mc <server>`):** Live player count, MOTD, and version check via mcstatus.io API.
* **`plugin_url_titler` & `plugin_safety`:** Automatically fetches OpenGraph titles and scans against malicious/homoglyph URLs.
* **`plugin_weather` (`!weer <city>`, `!weather`):** Open-Meteo weather with proactive `ApiQuotaGovernor` protection and 10-minute caching.
* **`plugin_crypto` (`!crypto <coin>`, `!currency <amt> <from> <to>`):** Live CoinGecko rates and European Central Bank FX conversions.

### Category 8: Moderation, Logging & Observability
* **`plugin_errors` (`!errors [count|clear]`):** Thread-safe ringbuffer of recent warnings and caught plugin errors.
* **`plugin_health` (`!uptime`, `!ping`, `!status`):** Latency and memory metrics across IRC, Discord, and FreeToken.
* **`plugin_raid_guard` & `plugin_flood_guard`:** Anti-clone protection and token-bucket flood prevention.
* **Structured Tracing Spans:** Tracing across `ircord::bridge`, `ircord::irc`, `ircord::discord`, `ircord::ai`, and `ircord::plugins`.
* **Audit Trail Logging:** Records all administrative actions in SQLite `audit_log`.
* **Metrics Endpoint (`GET /metrics`, `GET /health`, `GET /api/errors` on `:9090`):** Lightweight Prometheus and JSON health APIs.

---

## 7. Plugin Dependency Graph & Initialization Order

The Plugin Manager initializes subsystems in a strict dependency order:

```text
┌────────────────────────────────────────────────────────┐
│           Layer 1: Storage & Data Pool                 │
│   SQLite Connection Pool (sqlx) & Migrations           │
└──────────────────────────┬─────────────────────────────┘
                           │
┌──────────────────────────▼─────────────────────────────┐
│           Layer 2: Network & Protocol Adapters         │
│   Tokio MPSC Router, IRC TLS Stream, Discord Gateway   │
└──────────────────────────┬─────────────────────────────┘
                           │
┌──────────────────────────▼─────────────────────────────┐
│           Layer 3: Core AI & Quota Infrastructure      │
│   FlashML FreeToken Client & ApiQuotaGovernor          │
└──────────────────────────┬─────────────────────────────┘
                           │
┌──────────────────────────▼─────────────────────────────┐
│           Layer 4: Business Plugins & Rhai Engine      │
│   WhatPulse, Weather, Feeds, Reminders, Moderation     │
└────────────────────────────────────────────────────────┘
```

---

## 8. Database Schema (`migrations/20260908_init.sql`)

The database utilizes SQLite with WAL mode enabled:
* `presence`: Tracks last seen, last spoke, events, and quit reasons across IRC and Discord.
* `whatpulse_links`: Maps user nicknames to their WhatPulse profile names.
* `memos`: Persistent store for `!tell` offline messages.
* `feeds` & `feed_subscriptions`: Handles public and private RSS/Atom feeds and keyword filters.
* `user_tracks`: Keyword trackers for automatic DM alerts.
* `audit_log`: Administrative audit trail.
* `aliases`: Custom channel command shortcuts.
* `polls` & `poll_votes`: Active voting polls and recorded votes.
* `topic_history`: Historical log of channel topics.
* `chat_history`: FTS5 virtual table indexing channel history for AI RAG queries.

---

## 9. Performance Profile & Benchmarks

Thanks to zero-overhead asynchronous Rust, the daemon maintains an exceptionally small memory footprint:

| Scenario / Workload | RAM Usage | CPU Usage | Latency Impact |
| :--- | :--- | :--- | :--- |
| **Idle State (Connected to IRC & Discord)** | ~7 - 10 MB | 0.0% - 0.1% | N/A |
| **Normal Chat (1-3 active channels)** | ~11 - 14 MB | < 0.2% | < 3 ms per bridge event |
| **Heavy Load (10 bridged channels)** | ~15 - 18 MB | < 0.5% | < 5 ms per bridge event |
| **Active AI Generation (FreeToken streaming)** | ~16 - 20 MB | < 0.8% | Bound by FreeToken local LLM |
| **VLM Image Analysis (Vision)** | ~18 - 22 MB | < 1.0% | Inference workload offloaded to VLM process |

*Note: Even during traffic spikes, IRCord reliably uses less than 25 MB RAM, making it suitable for low-cost VPS instances or single-board computers (Raspberry Pi).*
