# IRCord 🤖⚡

> **High-Performance Hybride IRC-Discord AI Bot Daemon in Rust**

[![Rust](https://img.shields.io/badge/rust-2021_edition-orange.svg)](https://www.rust-lang.org/)
[![Tokio](https://img.shields.io/badge/async-tokio_1.40-blue.svg)](https://tokio.rs/)
[![Serenity](https://img.shields.io/badge/discord-serenity_0.12-5865F2.svg)](https://github.com/serenity-rs/serenity)
[![SQLite](https://img.shields.io/badge/database-sqlite_fts5-003B57.svg)](https://www.sqlite.org/)
[![FlashML](https://img.shields.io/badge/ai-FlashML_FreeToken-green.svg)](https://github.com/FlashML-org/FreeToken)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-lightgrey.svg)]()

> 🌐 **[View this documentation in English (README.md)](README.md)**

---

## 📖 Inhoudsopgave
- [Overzicht](#-overzicht)
- [Kernfunctionaliteiten](#-kernfunctionaliteiten)
- [Architectuur](#-architectuur)
- [Beschikbare Plugins & Commando's](#-beschikbare-plugins--commando's)
- [Edge-Native AI & RAG](#-edge-native-ai--rag)
- [Snel aan de slag](#-snel-aan-de-slag)
- [Configuratie](#-configuratie)
- [Projectstructuur](#-projectstructuur)
- [Installatie & Deployment](#-installatie--deployment)
- [Licentie](#-licentie)

---

## 🚀 Overzicht

**IRCord** is een modulaire, type-safe en uiterst zuinige daemon geschreven in **Rust**. De applicatie slaat een naadloze brug tussen klassieke **IRC-kanalen** (Internet Relay Chat) en moderne **Discord-servers**. 

Geïnspireerd door klassieke bots zoals [CloudBot](https://github.com/TotallyNotRobots/CloudBot) en [IRCPlus](https://github.com/Cjefke/IRCPlus), combineert IRCord rijke community-functionaliteiten met de kracht van **lokale edge-AI** via [FlashML FreeToken](https://github.com/FlashML-org/FreeToken).

### Waarom IRCord?
- 🦀 **Volledig in Rust:** Maximale betrouwbaarheid, geen garbage collection pauses en gegarandeerde thread-safety.
- 🪶 **Minimale Footprint:** Actief geheugengebruik onder de **20 MB RAM**.
- 🛡️ **Robuust & Panic-Proof:** Plugins draaien binnen geïsoleerde `catch_unwind` grenzen; een crash in één plugin brengt de daemon nooit ten val.
- 🔒 **Privacy-First AI:** Lokale LLM-inferentie (zoals DeepSeek of Qwen) via FreeToken; geen data naar externe commerciële cloud-API's.

---

## ✨ Kernfunctionaliteiten

1. **Bidirectionele Bridge (IRC ⇄ Discord)**
   - Berichtsynchronisatie in real-time.
   - Discord Webhook dispatching met dynamische avatars en bijpassende gebruikersnamen per IRC-gebruiker.
   - Weergave van Discord 'Reply-to' context op IRC: `<(Discord) Jan ↳ Piet>: Zeker, dat werkt!`
   - Anti-ping beveiliging (zero-width spaces tussen karakters van nicks) om ongewenste Discord mentions te voorkomen.
   - LRU Deduplicatie Cache om oneindige relay-loops waterdicht te blokkeren.

2. **Multi-Channel & Multi-Server Matrix**
   - Eén enkele daemon kan willekeurig veel IRC-kanalen en Discord-kanalen paarsgewijs koppelen.
   - Dynamische configuratie-validatie (`config.toml`).

3. **Modulair Plugin Systeem**
   - 25 ingebouwde native plugins voor community, moderatie, statistieken, media, gaming en AI.
   - Ondersteuning voor dynamische **[Rhai](https://rhai.rs/) scripts** in `./scripts/` voor live commando's zonder hercompileren.

4. **Lokale Edge-AI & Vision**
   - Directe integratie met de **FlashML FreeToken** OpenAI-compatibele API (`/v1/chat/completions`).
   - RAG (Retrieval-Augmented Generation) op basis van **SQLite FTS5** chatgeschiedenis.
   - Automatische Alt-Text generatie voor Discord afbeeldingen naar IRC.

5. **Community Statistieken: WhatPulse ("Team de Apen")**
   - Live statistieken voor toetsaanslagen, muisklikken en teamrankings.
   - Ondersteuning voor zowel de officiële WhatPulse API als custom REST endpoints (zoals `grandmasg.nl`).
   - Gebruikers kunnen hun IRC/Discord-nick koppelen aan hun WhatPulse profiel.

6. **Beveiliging & Moderatie**
   - IRCv3 SASL authenticatie (veilig inloggen vóór kanaaljoin, vereist voor `+r` kanalen).
   - Ingebouwde flood guard met instelbare delays en byte limits.
   - Anti-raid en clone-join bescherming.
   - Automatische codeblok-pastebin threshold (> 4 regels worden omgezet in een link).
   - Real-time token/secret lekdetectie via de `SafetyPlugin`.

7. **Observability & Webhooks**
   - Ingebouwde Axum HTTP server op poort `9090`.
   - `/health` endpoint voor Docker healthchecks.
   - `/metrics` endpoint voor monitoring.
   - `/api/github` webhook endpoint met HMAC SHA-256 handtekeningvalidatie.

---

## 🏛️ Architectuur

```text
┌────────────────┐      TLS Socket (Async)     ┌────────────────────────────────────────────────────────┐
│   IRC Server   │ ◄─────────────────────────► │                  Rust Daemon (Tokio)                   │
│ (Libera/Eigen) │                             │                                                        │
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

## 🧩 Beschikbare Plugins & Commando's

Alle commando's werken met zowel een uitroepteken (`!`) als een punt (`.`):

| Plugin | Triggers | Beschrijving | Voorbeeld |
|---|---|---|---|
| **WhatPulse** | `!wp`, `!whatpulse` | Haalt team- of individuele statistieken op van WhatPulse. Koppel nicks via `!wp link`. | `!wp`, `!wp Kuuke`, `!wp link Kuuke` |
| **AI Suite** | `!ai` | Stelt een vraag aan de lokale FreeToken LLM met kanaalcontext. | `!ai Leg uit wat een monad is.` |
| | `!ai models` | Toont het actieve model en alle beschikbare modellen op de lokale AI-server. | `!ai models` |
| | `!ai model <naam>` | **(Admin/Operator only)** Wisselt direct live het actieve AI-model zonder herstart. | `!ai model qwen2.5-coder` |
| | `!tldr [url]` | Samenvatting van recente chatgeschiedenis óf een webpagina-URL in 2 bullets. | `!tldr https://tweakers.net/...` |
| | `!catchup [aantal]` | Persoonlijke samenvatting van recente chatberichten bij terugkeer (afspraken, pings, highlights). | `!catchup`, `!catchup 50` |
| | `!vibe`, `!sentiment` | Peilt in 1-2 zinnen de sfeer en heetste gespreksonderwerpen in het kanaal. | `!vibe` |
| | `!roast <nick>` | Genereert een gevatte, speelse en humoristische nerd-roast in IRC-stijl. | `!roast Botje` |
| | `!whatis <begrip>` | Vlijmscherpe, nuchtere definitie van 1 regel voor een technisch begrip of term. | `!whatis BGP`, `!whatis Docker` |
| | `!topic suggest` | Laat AI een creatief nieuw kanaaltopic voorstellen. | `!topic suggest` |
| **Vertalen** | `!tr`, `!translate`, `!vertaal` | Vertaalt zinnen naar het Nederlands of een opgegeven doeltaal via lokale AI / MyMemory. | `!tr How is the weather?`, `!tr en:de Hallo` |
| **Presence** | `!seen`, `!lastonline` | Toont wanneer een gebruiker voor het laatst actief was en wat diens laatste actie was. | `!seen Klaas` |
| | `!online` | Toont een overzicht van actieve gebruikers op IRC en Discord. | `!online` |
| **AFK** | `!afk` | Schakelt AFK status in. Geeft automatisch antwoord wanneer iemand je noemt. | `!afk Even koffie halen` |
| **Weer** | `!weer`, `!weather` | Haalt live weersinformatie en temperatuur op via Open-Meteo. | `!weer Amsterdam` |
| **Slap** | `!slap`, `!mep` | De klassieke IRC forel-slap in moderne stijl. | `!slap Botje` |
| **Quotes** | `!quote`, `!q` | Bewaar en herinner legendarische kanaalquotes. | `!quote add <tekst>`, `!quote random` |
| **Karma** | `!karma`, `++`, `--` | Houd karma-scores bij voor onderwerpen en nicks. | `rust++`, `bugs--`, `!karma rust` |
| **Poll** | `!poll` | Start een interactieve kanaalpeiling met meerkeuzeopties. | `!poll Pizza vanavond? \| Ja \| Nee` |
| **Remind** | `!remind`, `!remindme` | Stelt een timer-herinnering in. | `!remind 10m Pizza uit de oven!` |
| **Alias** | `!alias` | Beheer aangepaste kanaal-aliassen. | `!alias add docs https://rust-lang.org` |
| **Tell (Memos)** | `!tell`, `!memo`, `!note` | Laat een offline bericht achter voor iemand; wordt automatisch bezorgd zodra diegene spreekt. | `!tell Klaas vergeet vanavond de server-backup niet` |
| **YouTube** | `!yt`, `!youtube` | Zoekt YouTube video's op of inspecteert links automatisch via oEmbed (titel, uploader, link, omschrijving). | `!yt lofi hip hop beats` |
| **Google Search** | `!g`, `!google`, `!search` | Zoekt direct op het web met titel, beknopte snippet en link (DuckDuckGo / Google CSE). | `!g rust lang documentation` |
| **Wikipedia** | `!wiki`, `!wkp` | Zoekt een samenvatting op de Nederlandse Wikipedia (met Engelse fallback). | `!wiki Linux`, `!wiki en Alan Turing` |
| **Crypto & Valuta** | `!crypto`, `!coin` | Actuele cryptokoersen in EUR & USD + 24u verandering (CoinGecko). | `!crypto btc`, `!crypto eth`, `!crypto sol` |
| | `!valuta`, `!fx`, `!currency` | Live wisselkoersen omrekenen via de Europese Centrale Bank. | `!valuta 100 usd eur`, `!valuta 50 gbp naar eur` |
| **Urban Dictionary**| `!ud`, `!urban` | Zoekt straattaal, slang en internet-definities inclusief praktijkvoorbeeld. | `!ud yeet`, `!ud poggers` |
| **Minecraft Status**| `!mc`, `!minecraft` | Pingt een Minecraft Java server voor online status, actuele spelers en MOTD. | `!mc play.hypixel.net` |
| **Wereldtijd** | `!tijd`, `!time`, `!klok` | Toont de actuele lokale tijd en datum in een wereldstad of land. | `!tijd Tokyo`, `!tijd New York` |
| **Verjaardagen** | `!bday`, `!verjaardag` | Registreer verjaardagen (`!bday set DD-MM[-JJJJ]`) en bekijk naderende verjaardagen (`!bday next`). De bot feliciteert jarigen automatisch 's ochtends met leeftijd en feestelijke felicitatie! | `!bday set 24-09`, `!bday next`, `!bday Klaas` |
| **Admin & Logs** | `!status`, `!ping`, `!stats` | Geeft uptime, actieve plugins, database- en geheugenstatistieken weer. | `!status` |
| | `!errors`, `!errorlog` | Toont de laatste waarschuwingen, plugin-fouten of panics (operators/owner). Gebruik `!errors clear` om te legen. | `!errors 5` |
| **Identiteit & Bridge**| `!link`, `!whois` | Koppel IRC nick en Discord account met 6-cijferige OTP code. Bekijk profiel, gekoppelde identiteit, karma en verjaardag. | `!link @Klaas`, `!link verify 123456`, `!whois Klaas` |
| | `!bridge stats`, `!top` | Toont totale bridge statistieken (aantal berichten, verdeling IRC vs Discord) en top-chatters. | `!bridge stats` |
| **Sysadmin & NAS** | `!nas`, `!hw`, `!sysinfo` | Telemetrie van de Minisforum N5 Pro NAS: OS, CPU load, RAM-geheugen en status van de Radeon 890M GPU. | `!nas` |
| | `!dns <domein> [type]` | Snelle DNS resolve via trustless DNS-over-HTTPS (DoH). Ondersteunt A, AAAA, MX, TXT, CNAME. | `!dns tweakers.net A` |
| | `!ssl <domein>`, `!http <url>`| Controleert HTTPS/TLS handshake en HSTS, of meet HTTP responstijd en statuscode (met SSRF-beveiliging). | `!ssl tweakers.net`, `!http https://site.nl` |
| **RSS Feeds** | `!rss add/list/del/latest`| Beheer RSS/Atom nieuwsfeeds. De achtergrondtaak pollt automatisch elke 10 minuten en plaatst nieuws direct in de chat. | `!rss add https://tweakers.net/feeds/nieuws.xml #algemeen` |
| **Tech & GitHub** | `!gh`, `!github <repo>` | Haalt sterren, open issues, omschrijving en actuele release op via de officiële GitHub REST API. | `!gh rust-lang/rust` |
| | `!cve`, `!security <id>` | Zoekt kwetsbaarheden en CVSS-beveiligingsscores op via de officiële OSV.dev / NIST database. | `!cve CVE-2024-3094` |
| **Sed (Passief)** | `s/oud/nieuw/` | Corrigeert automatisch typefouten uit je vorige bericht. | `s/fout/goed/` |
| **URL Titler (Passief)** | *Automatisch* | Detecteert URL's in chat en toont direct de `<title>` van de pagina. | `https://github.com/...` |
| **Safety (Passief)** | *Automatisch* | Waarschuwt direct wanneer iemand per ongeluk API keys of tokens lekt. | — |
| **Reactions (Passief)**| *Automatisch* | Reageert op specifieke trefwoorden en begroetingen. | — |

---

## 🧠 Edge-Native AI & RAG

IRCord vereist geen dure cloud tokens. De bot is ontworpen om te communiceren met een lokale instantie van **FlashML FreeToken**:
- **OpenAI-compatibel:** Maakt gebruik van de `/v1/chat/completions` specificatie.
- **RAG via SQLite FTS5:** Bij commando's zoals `!tldr` en `!ai` doorzoekt de daemon automatisch de lokale full-text search index om relevante historische context op te halen.
- **Vision:** Binnenkomende afbeeldingen op Discord worden verwerkt en via een beknopte Alt-Text omschrijving gedeeld op IRC.
- **Token Budget Guard:** Beveiligd tegen oneindige loops door een configureerbaar tokenbudget per uur (`hourly_token_budget`).

---

## ⚡ Snel aan de slag

### 1. Repository klonen
```bash
git clone https://github.com/jouw-organisatie/ircord.git
cd ircord
```

### 2. Configuratie klaarzetten
Kopieer het voorbeeldbestand voor omgevingsvariabelen en pas deze aan:
```bash
cp .env.example .env
cp config.toml config.toml
```

Vul minimaal de volgende waarden in je `.env` in:
- `DISCORD_BOT_TOKEN`: Token van je Discord bot.
- `IRC_SERVER` & `IRC_NICK`: Jouw gewenste IRC netwerk en nick.

En koppel je kanalen in `config.toml`:
```toml
[[channels]]
irc_channel = "#mijnkanaal"
discord_channel_id = 123456789012345678
discord_webhook_url = "https://discord.com/api/webhooks/..."
```

### 3. Starten met Docker Compose (Kies je gewenste versie) 🐳

Je kunt kiezen uit drie aparte Docker Compose bestanden, of gebruik maken van het interactieve start-menu:

#### Optie A: Interactief Startmenu (Makkelijkst!)
- **Windows**: Dubbelklik op [`start.bat`](file:///d:/IRCord/start.bat) of start in PowerShell.
- **Linux / NAS (Minisforum N5 Pro)**: Voer `./start.sh` uit in je terminal.
Kies simpelweg `1`, `2` of `3` in het keuzemenu.

#### Optie B: Direct per Compose-bestand starten
- **Versie 1: Ollama ROCm + GPU (Aanbevolen voor Minisforum N5 Pro)**
  *Start Ollama met Radeon 890M GPU acceleratie en downloadt automatisch het AI-model (bijv. Qwen 2.5)*:
  ```bash
  docker compose -f docker-compose.ollama.yml up -d --build
  # of gewoon 'docker compose up -d --build'
  ```

- **Versie 2: FreeToken ROCm**
  *Start de FlashML FreeToken AI inference server*:
  ```bash
  docker compose -f docker-compose.freetoken.yml up -d --build
  ```

- **Versie 3: Standalone Bot Daemon**
  *Start alleen de IRCord daemon (lichtgewicht, verbindt met externe/host Ollama)*:
  ```bash
  docker compose -f docker-compose.standalone.yml up -d --build
  ```

Volg de status en logs via:
```bash
docker compose ps
docker compose logs -f ircord
docker compose logs -f ircord-ollama-init
```

---

## ⚙️ Configuratie

Het bestand `config.toml` regelt alle gedragsparameters van de bot:

```toml
[general]
language = "nl" # Keuze: "nl" of "en"
bot_owner_discord_id = 0
bot_owner_irc_nick = "Kuuke"
http_port = 9090
pastebin_threshold_lines = 4
admin_channel_irc = "#bot-logs"
admin_channel_discord_id = 0 # Optioneel: Discord kanaal-ID voor #bot-logs

[bridge]
loop_prevent_timeout_sec = 10
lru_cache_capacity = 2000
sync_presence = true
sync_edits = true

[[channels]]
irc_channel = "#algemeen"
discord_channel_id = 123456789012345678
discord_webhook_url = "https://discord.com/api/webhooks/..."

[whatpulse]
team_name = "Team de Apen"
api_url = "https://whatpulse.org/api/v1" # Zie https://whatpulse.org/help/api/web/intro
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

## 📂 Projectstructuur

```text
IRCord/
├── migrations/                # SQLite database migraties (schema & FTS5)
│   └── 20260908_init.sql
├── scripts/                   # Hot-reloadbare Rhai scripts voor community commando's
│   ├── hello.rhai             # Meertalig begroetingsscript (Nederlands/Engels/Duits)
│   └── hallo.rhai             # Nederlands begroetingsscript
├── src/
│   ├── ai/                    # FreeToken client, RAG searcher & Vision helpers
│   ├── bridge/                # Bridge router, normalisatie & deduplicatie cache
│   ├── config.rs              # TOML configuratie parser & validatie
│   ├── discord/               # Serenity Gateway handler & Webhook dispatcher
│   ├── irc/                   # IRC client taak, SASL handling & flood guard
│   ├── plugins/               # 25 modulaire plugins (WhatPulse, AI, Moderatie, etc.)
│   ├── utils/                 # Lifecycle, graceful shutdown & helpers
│   ├── web/                   # Axum HTTP server (/health, /metrics, GitHub webhooks)
│   └── main.rs                # Daemon opstartprocedure & event loops
├── docs/
│   ├── DESIGN_SPEC.md         # Oorspronkelijke architectuurspecificatie en ontwerpplan (Engels)
│   └── DESIGN_SPEC.nl.md      # Oorspronkelijke architectuurspecificatie en ontwerpplan (Nederlands)
├── .env.example               # Voorbeeld omgevingsvariabelen (Engels)
├── .env.nl.example            # Voorbeeld omgevingsvariabelen (Nederlands)
├── Cargo.toml                 # Rust dependencies & package metadata
├── config.toml                # Applicatieconfiguratie
├── Dockerfile                 # Multi-stage Alpine container voor minimale footprint
├── Dockerfile.freetoken       # FlashML FreeToken AI inference container
├── docker-compose.yml         # Volledige stack orchestratie
├── INSTALL.md                 # Uitgebreide installatie- en deploymenthandleiding
├── README.nl.md               # Nederlandse documentatie
└── README.md                  # Hoofddocumentatie (Engels)
```

---

## 📚 Installatie & Deployment

Voor een diepgaande stap-voor-stap installatiehandleiding (inclusief Discord Bot Portal setup, SASL configuratie, bare-metal installatie en systemd service setup), raadpleeg:

👉 **[INSTALL.md](INSTALL.md)**

---

## 📄 Licentie

Gedistribueerd onder de **MIT** of **Apache 2.0** licentie. Zie de broncode voor nadere details.
