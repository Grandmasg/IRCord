# Project- & Architectuurplan: Hybride IRC-Discord AI Bot (Rust Edition)

> [!NOTE]
> **Archived Design Specification & RFC**  
> Dit document bevat de oorspronkelijke architectuurblauwdruk en ontwerpspecificatie van **IRCord**.  
> Voor actuele documentatie, commando's en installatiehandleidingen verwijzen we naar:
> - [README.md](../README.md) (Officiële hoofddocumentatie)
> - [README.nl.md](../README.nl.md) (Nederlandstalige gebruikershandleiding)
> - [INSTALL.md](../INSTALL.md) (Installatie & deployment handleiding)

## 1. Visie & Doelstelling
Het doel is het ontwikkelen van een **high-performance, modulaire en type-safe bot-daemon** in **Rust**. De bot overbrugt klassieke Internet Relay Chat (IRC) kanalen met moderne Discord-servers en combineert de rijke functionaliteit van klassieke IRC-scripts (zoals [Cjefke/IRCPlus](https://github.com/Cjefke/IRCPlus) en [TotallyNotRobots/CloudBot](https://github.com/TotallyNotRobots/CloudBot)) met lokale edge-AI via **[FlashML FreeToken](https://github.com/FlashML-org/FreeToken)**.

Belangrijkste kernfunctionaliteiten:
1. **Bidirectionele Relais (Bridge)**: Naadloze berichtenstroom tussen IRC en Discord met webhook-avatars, anti-ping zero-width spaties, Discord Reply-to-IRC contextweergave en cross-platform mention resolving.
2. **Multi-Channel & Multi-Server Support**: Eén enkele daemon die meerdere kanaalparen tegelijk synchroniseert via gevalideerde configuratie met live reload.
3. **Modulair Plugin Systeem (CloudBot & IRCPlus Filosofie)**: Uitbreidbare plugin-architectuur via een asynchrone Rust Trait én gesandboxte hot-reload scripting via [Rhai](https://rhai.rs/) voor community-commando's zonder hercompileren.
4. **Lokale Edge-AI**: Aanroepen van lokale LLM's via FlashML FreeToken (`/v1/chat/completions`) voor privacy, zero cloud-kosten en edge-inferentie op consumentenhardware (CPU/GPU co-executie).
5. **AI Vision & Image Alt-Text**: Automatische beknopte omschrijvingen van Discord-afbeeldingen en screenshots voor IRC-gebruikers (met embedding/resultaat caching).
6. **Slimme AI-Toepassingen**: `!tldr` (gesprekssamenvatting), `!topic suggest`, AI-vertaling en SQLite FTS5 RAG op de kanaalgeschiedenis.
7. **Presence, AFK & Last Online Hub**: Hybride aanwezigheidsdetectie (`!online`, `!lastonline`, `!seen`, quit-redenen en AFK-notificaties).
8. **WhatPulse Community Stats ("Team de Apen")**: Live teamstatistieken (toetsaanslagen, muisklikken, ranking en persoonlijke stats) via de officiële WhatPulse API en de custom API op `https://www.grandmasg.nl/WPNEW/`.
9. **Persoonlijke & Publieke Feeds (RSS/Atom & Keywords)**: Nieuwsfeeds in het kanaal én op maat gemaakte persoonlijke abonnementen via DM/Query met trefwoord-alerts en AI-digests.
10. **Security & Abuse-Hardening**: RBAC-rechtenmodel (Ops/Mods/Owner), anti-raid/clone guard, flood guard, auto-pastebin voor codeblokken (>4 regels), NickServ spoofing-preventie en GitHub HMAC-handtekeningvalidatie.
11. **Enterprise Observability & Graceful Lifecycle**: Gestructureerde tracing spans, gecontroleerde zero-data-loss shutdown flow en voorspelbare memory-footprint (< 20 MB RAM).

---

## 2. Systeem- en Plugin-Architectuur

Het hart van de bot is een centrale eventbus die inkomende events van zowel IRC als Discord normaliseert en doorstuurt naar de **Plugin Manager**:

```text
┌────────────────┐      TLS Socket (Async)     ┌────────────────────────────────────────────────────────┐
│   IRC Server   │ ◄─────────────────────────► │                  Rust Daemon (Tokio)                   │
│ (Libera/Eigen) │                             │                                                        │
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
                                                        │    │  https://www.grandmasg.nl/WPNEW/         │
                                                        │    │  (WhatPulse API - Team de Apen)          │
                                                        │    └──────────────────────────────────────────┘
                                                        ▼
                                               ┌────────────────────────────────────────────────────────┐
                                               │                FlashML FreeToken Engine                │
                                               │        (Local Edge MoE: DeepSeek / Qwen / etc.)        │
                                               └────────────────────────────────────────────────────────┘
```

---

## 3. Geavanceerde Bridge-UX, Multi-Channel & Formatting

### 3.1 Multi-Channel Mapping & Config Validatie met Live Reload
Eén daemon kan meerdere kanalen tegelijk bedienen via een matrix-mapping in `config.toml`.
* **Schema-Validatie:** Bij het parsen via `serde` valideert de bot:
  * Geen dubbele IRC-kanalen of dubbele Discord-kanalen.
  * Formaatvalidatie van webhook-URL's (`https://discord.com/api/webhooks/...`).
* **Live Reload:**
  * Via een achtergrondtaak met `notify` (bestands-watcher) of een `SIGHUP` Unix-signaal wordt `config.toml` opnieuw ingelezen.
  * Gewijzigde kanaalparen of trefwoorden worden dynamisch geüpdatet zonder dat de actieve IRC TLS-socket of Discord Gateway-verbinding hoeft te herstarten.

### 3.2 Discord Replies naar IRC Context
Wanneer een Discord-gebruiker reageert via de officiële "Reply"-knop op een specifiek vorig bericht:
* Op IRC wordt dit expliciet geparseerd:  
  `<(Discord) Pietje ↳ Klaas>: Dat klopt inderdaad!`  
  Hierdoor blijft de context op IRC glashelder, zelfs in drukke gesprekken.

### 3.3 Automatische Pastebin voor Lange Codeblokken
* Als een bericht of commando-output meer dan 4 regels telt, uploadt de bot de content automatisch naar een lichtgewicht pastebin (bijv. `0x0.st`) en stuurt naar IRC:  
  `* [Codeblock: 32 regels] https://0x0.st/xyz.txt`

### 3.4 Discord Mentions & mIRC Color Codes
* `<@123456789>` wordt omgezet naar `@Gebruikersnaam`.
* `<#987654321>` wordt omgezet naar `#kanaalnaam`.
* Custom emoji's `<:naam:12345>` worden `:naam:`.
* mIRC kleurcodes (`\x03`, `\x02`, `\x1f`) worden gestript voor Discord transmissie.
* Zero-width space (`\u{200B}`) wordt ingevoegd in de auteur-prefix op IRC om ongewenste highlights te voorkomen.

### 3.5 Bridge Consistency, Loop-Preventie & Message-ID Mapping
Om oneindige feedback-loops (`IRC -> Discord -> IRC`) waterdicht te voorkomen en reply-context nauwkeurig te behouden:
* **Loop-Guard Ingress Filter:**
  * Discord webhook-berichten dragen een `webhook_id`; deze worden bij binnenkomst direct gedropt door de gateway-listener.
  * Berichten verzonden door de eigen bot-ID worden op beide platformen genegeerd.
* **Message-ID Cache (Ringbuffer / LRU Cache):**
  * De laatste 2.000 verzonden bridge-berichten worden bijgehouden in een in-memory LRU-cache (`Discord Message ID <-> IRC Nick + Timestamp`).
  * Als een Discord-gebruiker op een bericht reageert met een native Reply, zoekt de bot het oorspronkelijke bericht op in de cache om te bepalen wie de geadresseerde was (`Pietje ↳ Klaas`).
* **Optionele Message Edit Sync:**
  * Als een gebruiker op Discord binnen 60 seconden een bericht bewerkt, stuurt de bot optioneel een subtiele correctie naar IRC:  
    `* [Edit] Pietje: <aangepaste tekst>`.

### 3.6 Discord Rate-Limit & Webhook Retry-Strategie
* **Rate-Limit Buckets:** Serenity beheert interne Discord REST rate-limits via token-buckets per route.
* **Webhook Retry Backoff:**
  * Uitgaande webhook-posts naar Discord worden verwerkt via een interne MPSC-queue.
  * Bij een `HTTP 429 Too Many Requests` leest de daemon de `retry_after` header uit en pauzeert de specifieke webhook-worker met exponential backoff (max 3 retries), zonder de IRC-loop te blokkeren.

### 3.7 Plugin Lifecycle & Error-Isolatie (Fault Tolerance)
* **Panic Boundaries (`catch_unwind`):**
  * Elke plugin-aanroep (`on_command`, `on_message`) wordt gewrapped in een `tokio::task::spawn` met `std::panic::AssertUnwindSafe`.
  * Mocht een plugin onverhoopt panicken (bijv. door een onverwachte regex-fout of string-slice index buiten bereik), dan crasht **alleen die specifieke plugin-taak**, terwijl de bridge, IRC-verbinding en overige plugins 100% blijven doordraaien.
* **Health & Restart Policy:**
  * Panics worden gelogd via gestructureerde tracing (`tracing::error!`) en gerapporteerd in `!status`.
  * Geheugen van falende taken wordt direct vrijgegeven door Tokio.

### 3.8 Graceful Shutdown Flow (Zero Data Loss)
Bij een `SIGINT` (Ctrl+C), `SIGTERM` (Docker stop) of een `!shutdown` commando van de Bot Owner:
1. **Broadcast Cancellation:** Een `tokio_util::sync::CancellationToken` signaleert alle actieve taken om nieuwe inkomende events te weigeren.
2. **Queue Draining:** De uitgaande webhook- en pastebin-wachtrijen krijgen 5 seconden de tijd om resterende berichten netjes af te leveren.
3. **IRC Protocol Afsluiting:** De bot verzendt een officiële `QUIT :IRCord daemon gracefully shutting down...` naar de IRC-server en sluit de TLS-stream.
4. **Discord Gateway Sluiting:** Serenity ontkoppelt de WebSocket-shard netjes van Discord.
5. **Database WAL Checkpoint:** De SQLite connection pool sluit alle verbindingen en dwingt een WAL-checkpoint af (`pool.close().await`), zodat data 100% consistent op schijf staat.

---

## 4. Rechten- & Rollensysteem (RBAC) & Security Hardening

### 4.1 RBAC Levels
Om beheer veilig te houden over beide netwerken heen:
* **Level 0 (Public):** Standaard commando's (`!wp`, `!weer`, `!calc`, `!seen`, `!online`, persoonlijke feeds via DM, `!ai` met normale cooldown).
* **Level 1 (Trusted / Voiced `+v` / Actieve Discord-leden):** Snellere AI-cooldown, toegang tot `!tldr` en `!quote add`.
* **Level 2 (Channel Ops `@` / Discord Moderators):** `!topic`, `!kick`, `!silence`, `!paste`, publieke kanaal-feeds toevoegen (`!rss add`).
* **Level 3 (Bot Owner):** `!reload scripts`, `!setmodel`, `!metrics`, `!shutdown` (gevalideerd via NickServ-accountnaam op IRC of vaste Discord User ID).

### 4.2 Security & Abuse-Hardening
* **IRC Nick-Spoofing Preventie:**
  * Voor gevoelige opdrachten (Level 2 en 3) vertrouwt de bot niet blind op de nick van de afzender, maar verifieert hij asynchroon de NickServ identificatiestatus (`STATUS <nick>` of `ACC <nick>`).
* **SQL Injection Veiligheid:**
  * Alle database-operaties lopen exclusief via `sqlx` geparametriseerde queries (`query!`, `query_as!`). String-concatenatie van SQL is strikt verboden.
* **Rhai Script Sandboxing:**
  * De embedded Rhai scripting engine wordt geconfigureerd met strikte resource-limieten:
    * `engine.set_max_operations(50_000)` (voorkomt oneindige lussen).
    * `engine.set_max_string_size(1_000)` (voorkomt memory exhaustie).
    * Geen toegang tot het bestandssysteem of netwerk vanuit Rhai.
* **Anti-Impersonation:**
  * Discord Webhooks worden voorzien van strikte sanitization op usernames om te voorkomen dat IRC-gebruikers zich voordoen als serverbeheerders.

---

## 5. WhatPulse Integratie ("Team de Apen")

De WhatPulse-community is een oerklassieke pijler van de Nederlandse IRC-cultuur. De bot ondersteunt **twee flexibele bronnen**:
1. **Directe Officiële WhatPulse Web API v1** (`https://whatpulse.org/api/v1/`, zie [WhatPulse API Documentatie](https://whatpulse.org/help/api/web/intro)) met Bearer authenticatie via `WHATPULSE_API_KEY`.
2. **Eigen Aggregator / Proxy Endpoint** (`https://www.grandmasg.nl/WPNEW/`) voor realtime pulse-diffs, custom team leaderboards en caching.

### 5.1 Commando's
* **`!wp` / `!whatpulse`:** Toont actuele statistieken van **Team de Apen** (Keys, clicks, team rank, download/upload).
* **`!wp user [nick]` / `!wp <nick>` / `!wp me`:** Toont individuele statistieken van een gekoppelde gebruiker (of jezelf).
* **`!wp link <whatpulse_gebruikersnaam>`:** Koppelt je IRC-nick of Discord-account aan je WhatPulse profiel.
* **`!wp top`:** Toont het leaderboard van de top 5 typers/clickers binnen Team de Apen.
* **Live Pulse Notificaties (Background Worker):**
  * Zodra Kuuke (of een ander teamlid) een pulse uploadt, verschijnt er direct een live kanaalbericht:  
    `* [WhatPulse] Kuuke heeft zojuist gepulset! +14.280 keys, +4.190 clicks (Totaal: 12.840.100 keys)`
  * **Inhaal-Alerts:** Als Kuuke door deze pulse een ander teamlid inhaalt op het leaderboard:  
    `* [WhatPulse] Ranglijst-update! Kuuke stijgt naar plek #3 in Team de Apen (passeert Pietje)!`
* **Mijlpaal-Alerts:** Zodra het team of een individu een ronde grens passeert (bijv. 10M keys).

### 5.2 Ondersteunde API Endpoints
* **Officiële WhatPulse Web API v1:**
  * Teams: `GET https://whatpulse.org/api/v1/teams?search=Team+de+Apen` en `GET /teams/:id`
  * Users: `GET https://whatpulse.org/api/v1/users?search=username` en `GET /users/:id`
  * Bearer token authenticatie (`WHATPULSE_API_KEY`)
* **Custom Backend (`https://www.grandmasg.nl/WPNEW/`):** JSON-feed met `team`, `recent_pulses` (met `keys_added` en `passed_user`) en `top_members`.

---

## 6. Complete Plugin Catalogus (CloudBot, IRCPlus & AI)

### Categorie 1: AI-Superkrachten & Model Management (FlashML FreeToken)
* **`plugin_ai` (`!ai <vraag>` of `@bot mention`):**
  * Directe vraagbaak met lokale inferentie via FreeToken (`/v1/chat/completions`).
  * Sliding window geheugen per kanaal (laatste 8 interacties).
  * Automatische regel-chunker voor IRC (max 380 bytes / 800ms delay).
* **AI Model Management & Token Budgeting:**
  * `!setmodel <modelnaam>` (Bot Owner): Schakelt dynamisch over naar een ander lokaal model met warmup ping en validatie.
  * Kanaal-Tokenbudget: Maximaal tokenverbruik per uur per kanaal om FreeToken niet te overbelasten.
* **`plugin_vision` (AI Afbeeldingsbeschrijving voor IRC met Caching):**
  * VLM-engine genereert een beknopte 1-regel omschrijving van Discord-afbeeldingen voor IRC.
  * Hash-caching voorkomt dubbele VLM-aanroepen voor identieke afbeeldingen.
* **`plugin_tldr` (`!tldr [laatste X berichten/minuten]` of `!summary`):**
  * Haalt recente chatlogs op uit SQLite en genereert een beknopte 3-punts samenvatting.
* **`plugin_topic` (`!topic suggest`):**
  * Analyseert de lopende discussie en stelt een passend kanaaltopic voor.
* **`plugin_translate` (`!tr <doeltaal> <tekst>`):**
  * Vertaalt tekst of foutmeldingen snel via de AI.
* **`plugin_rag_history` (`!ai wie zei wat over X?`):**
  * Doorzoekt de SQLite FTS5 tabel en gebruikt de context voor RAG-injectie.

### Categorie 2: Community Stats & WhatPulse
* **`plugin_whatpulse` (`!wp`, `!wp user`, `!wp link`, `!wp top`):**
  * Haalt gecachete statistieken op van `https://www.grandmasg.nl/WPNEW/` of de officiële API voor Team de Apen.
* **`plugin_stats` (`!top`, `!peak`):**
  * `!top`: Top 5 meest actieve chatters van vandaag/deze week.
  * `!peak`: Historisch recordaantal gelijktijdige online gebruikers.

### Categorie 3: Presence, AFK & Last Online Hub
* **`plugin_last_online` (`!lastonline <nick>` of `!seen <nick>`):**
  * Onderscheid tussen **laatst gesproken** en **laatst online** (inclusief quit-redenen).
* **`plugin_online` (`!online` of `!users`):**
  * Gecombineerd overzicht: `[Online] IRC (3): @Klaas, +Pietje, Jan | Discord (4): Anja (Actief), Bart (Idle), Dev (Voice 🔊)`.
* **`plugin_afk` (`!afk [reden]`):**
  * Afwezigheidsmelder met auto-notificatie bij mentions en automatisch herstel.
* **`plugin_seen_tell` (`!tell <nick> <boodschap>`):**
  * Cross-platform memo's afgeleverd zodra de ontvanger weer spreekt of online komt.

### Categorie 4: Nieuwsfeeds & Persoonlijke Notificaties
* **Publieke Kanaal-Feeds (`!rss add <url> [kanaal]`, `!rss list`):**
  * Plaatst nieuwe artikelen automatisch in het aangewezen kanaal.
* **Persoonlijke Feeds via DM / Query (`!rss sub <url>`):**
  * Abonnementen die exclusief via privébericht worden bezorgd.
* **Trefwoord-Alerts (`!track <zoekterm>`):**
  * Monitort feeds op specifieke termen en stuurt direct een alert via DM.
* **Persoonlijke AI Nieuws-Digest (`!digest`):**
  * Dagelijkse ochtendbriefing van 5 bullets samengesteld door FreeToken.

### Categorie 5: Klassieke IRC- & Eggdrop-Cultuur
* **`plugin_slap` (`!slap <target>`):**
  * IRC CTCP ACTION: `\x01ACTION slaps <target> around a bit with a large trout\x01`.
  * Discord: Cursieve rendering `*IRCord slaps <target> around a bit with a large trout*`.
* **`plugin_remind` (`!remindme <tijd> <bericht>`, `!remind channel <tijd> <tekst>`):**
  * Persoonlijke én kanaal-brede herinneringen (inclusief periodieke herhalingen voor wekelijkse standups).
  * Persistent opgeslagen in SQLite met Tokio background timers.
* **`plugin_quotes` (`!quote add <tekst>`, `!quote [zoekterm]`, `!quote random`):**
  * Quotes Database (QDB) voor memorabele kanaaluitspraken.
* **`plugin_karma` (`nick++`, `nick--`, `!karma <nick>`):**
  * Reputatietracker met beveiliging tegen zelf-stemmen.
* **`plugin_sed` (`s/oud/nieuw/` of `s/fout/goed/g`):**
  * Corrigeert typefouten in het voorgaande bericht van de afzender.
* **`plugin_dice` (`!roll [XdY]`, `!choose <a/b/c>`):**
  * Dobbelstenen en keuzehulpen.

### Categorie 6: Community Utility, Interactie & Bridges
* **`plugin_alias` (`!alias add !naam <tekst>`, `!alias del !naam`, `!alias list`):**
  * Laat gebruikers of moderators snelle custom commando's definiëren (opgeslagen in SQLite, met RBAC-restrictie).
* **`plugin_poll` (`!poll start "Vraag?" optie1/optie2`, `!poll vote <optie>`, `!poll end`):**
  * Hybride stempeilingen: Discord toont interactieve buttons/emojis, IRC-ers stemmen via `!poll vote <nr>`. Resultaten worden live gesynchroniseerd.
* **`plugin_reaction_bridge` (Emoji Reaction Relais):**
  * Wanneer iemand op Discord reageert met een emoji (bijv. 👍 of ❤️), meldt de bot dit discreet op IRC:  
    `* [Reactie] Pietje reageerde met :thumbsup: op bericht van Klaas`.
* **`plugin_auto_responder` (Regex FAQ & Trefwoord Snippets):**
  * Reageert discreet op specifieke patronen (bijv. *"hoe compileer ik"* -> link naar buildinstructies of tips).
* **`plugin_file_upload` (`!upload <bestand>`, `!img <url>`):**
  * Maakt het voor IRC-gebruikers mogelijk om afbeeldings-URL's direct als rijke Discord image-embed door te sturen.

### Categorie 7: Informatie, Feeds & URL Safety
* **`plugin_url_titler` & `plugin_url_safety` (Automatisch):**
  * Detecteert links, haalt asynchroon OpenGraph/HTML metadata op en toont preview.
  * **Anti-Phishing & Malware Guard:** Controleert URL's tegen bekende verdachte domeinen of homoglyph/lookalike domeinen en waarschuwt de chat.
* **`plugin_github` (Axum Webhook Receiver met HMAC):**
  * Ontvangt GitHub commit-, issue- en release-webhooks met `X-Hub-Signature-256` validatie.
* **`plugin_weather` (`!weer [locatie]`, `!weather`):**
  * Weersverwachting via Open-Meteo.
* **`plugin_crypto` (`!crypto <btc/eth/...>`):**
  * Realtime crypto-koersen via CoinGecko.
* **`plugin_calc` (`!calc <expressie>`):**
  * Veilige wiskundige rekenmachine.

### Categorie 8: Moderatie, Logging & Observability
* **`plugin_log_exporter` (`!log export [uren]`, `!log search <term>`):**
  * Exporteert recente kanaalchatlogs naar een pastebin (`0x0.st`) of doorzoekt de geschiedenis.
* **`plugin_topic_history` (`!topic history`, `!topic revert <id>`):**
  * Houdt een historisch archief bij van kanaaltopics en laat moderators eerdere topics met één commando herstellen.
* **`plugin_health` (`!uptime`, `!latency irc`, `!latency discord`, `!ping`):**
  * User-facing netwerk- en latency-statistieken voor IRC, Discord en de FreeToken engine.
* **`plugin_raid_guard` (Anti-Raid & Clone Guard):**
  * Schakelt tijdelijke mute (`+m`) in bij join-pieken of massale spam.
* **`plugin_flood_guard`:**
  * Token-bucket rate limiter voor IRC.
* **Structured Tracing Spans:**
  * Dedicated namespaces voor gestructureerde logging:
    * `ircord::bridge` (velden: `channel`, `direction`, `msg_id`, `latency_ms`)
    * `ircord::irc` (velden: `server`, `command`, `nick`)
    * `ircord::discord` (velden: `guild_id`, `channel_id`, `author_id`)
    * `ircord::ai` (velden: `model`, `prompt_tokens`, `completion_tokens`, `duration_ms`)
    * `ircord::plugins` (velden: `plugin`, `trigger`, `author`)
* **Audit Trail Logging:**
  * Alle moderatie-acties worden gelogd in de SQLite `audit_log` tabel.
* **`plugin_admin` (`!status`, `!reload`, `!stats`):**
  * Toont uptime, actueel RAM-verbruik (MB), plugin-latencies en WhatPulse cachestatus.
* **`metrics_endpoint` (HTTP server op `:9090`):**
  * Exposeert interne tellers en status voor monitoring via Prometheus of browser.

---

## 7. Plugin Dependency Graph & Initialisatie-Volgorde

Sommige plugins zijn afhankelijk van onderliggende systemen (bijv. `AI -> RAG -> SQLite FTS5`). De Plugin Manager volgt daarom een strikte initialisatie-volgorde via Dependency Injection:

```text
┌────────────────────────────────────────────────────────┐
│           Laag 1: Opslag & Datapool                    │
│   SQLite Connection Pool (sqlx) & Migraties            │
└──────────────────────────┬─────────────────────────────┘
                           │
┌──────────────────────────▼─────────────────────────────┐
│           Laag 2: Netwerk & Protocol Adapters          │
│   Tokio MPSC Router, IRC TLS Stream, Discord Gateway   │
└──────────────────────────┬─────────────────────────────┘
                           │
┌──────────────────────────▼─────────────────────────────┐
│           Laag 3: Core AI Engine                       │
│   FlashML FreeToken Client & VLM Hash Cache            │
└──────────────────────────┬─────────────────────────────┘
                           │
┌──────────────────────────▼─────────────────────────────┐
│           Laag 4: Business Plugins & Rhai Engine       │
│   WhatPulse, Presence, Feeds, Reminder, Rhai Hot-reload│
└────────────────────────────────────────────────────────┘
```

---

## 8. Database Schema & Lifecycle Maintenance (`migrations/20260908_init.sql`)

### 8.1 Schema
```sql
-- Gebruikersaanwezigheid en last-seen logging
CREATE TABLE IF NOT EXISTS presence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nick TEXT NOT NULL,
    platform TEXT NOT NULL, -- 'irc' of 'discord'
    last_seen_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_spoke_at TIMESTAMP,
    last_event TEXT,        -- 'join', 'part', 'quit', 'msg'
    quit_message TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_presence_nick_platform ON presence(nick, platform);

-- WhatPulse Nickname Koppelingen
CREATE TABLE IF NOT EXISTS whatpulse_links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nick TEXT NOT NULL,
    platform TEXT NOT NULL,
    whatpulse_username TEXT NOT NULL,
    linked_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_wp_nick_platform ON whatpulse_links(nick, platform);

-- Offline Memos (!tell)
CREATE TABLE IF NOT EXISTS memos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recipient TEXT NOT NULL,
    sender TEXT NOT NULL,
    platform TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    delivered_at TIMESTAMP
);

-- Persoonlijke en Publieke RSS Feeds
CREATE TABLE IF NOT EXISTS feeds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT UNIQUE NOT NULL,
    title TEXT,
    last_guid TEXT,
    last_checked_at TIMESTAMP
);

CREATE TABLE IF NOT EXISTS feed_subscriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    feed_id INTEGER REFERENCES feeds(id) ON DELETE CASCADE,
    target_type TEXT NOT NULL, -- 'channel' of 'user_dm'
    target_id TEXT NOT NULL,   -- kanaalnaam of user nick/ID
    platform TEXT NOT NULL,    -- 'irc' of 'discord'
    keyword_filter TEXT        -- optioneel trefwoordfilter
);

-- Persoonlijke Trefwoord-Trackers (!track)
CREATE TABLE IF NOT EXISTS user_tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    platform TEXT NOT NULL,
    keyword TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Audit Trail voor Moderatie & Beheer
CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operator TEXT NOT NULL,
    platform TEXT NOT NULL,
    action TEXT NOT NULL,
    details TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Custom Aliases (!alias)
CREATE TABLE IF NOT EXISTS aliases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trigger TEXT UNIQUE NOT NULL,
    response TEXT NOT NULL,
    creator TEXT NOT NULL,
    platform TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Kanaal-Peilingen / Polls (!poll)
CREATE TABLE IF NOT EXISTS polls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel TEXT NOT NULL,
    question TEXT NOT NULL,
    options_json TEXT NOT NULL, -- JSON array van keuzes
    is_active BOOLEAN DEFAULT TRUE,
    created_by TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS poll_votes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    poll_id INTEGER REFERENCES polls(id) ON DELETE CASCADE,
    voter TEXT NOT NULL,
    platform TEXT NOT NULL,
    option_index INTEGER NOT NULL,
    voted_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(poll_id, voter, platform)
);

-- Kanaaltopic Geschiedenis
CREATE TABLE IF NOT EXISTS topic_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel TEXT NOT NULL,
    topic TEXT NOT NULL,
    set_by TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Chatlog met FTS5 voor AI-RAG
CREATE VIRTUAL TABLE IF NOT EXISTS chat_history USING fts5(
    channel,
    author,
    platform,
    message,
    timestamp
);
```

### 8.2 Database Lifecycle, Pruning & Maintenance
* **Automatische Retentie:** Een wekelijkse achtergrondtaak verwijdert reguliere chatberichten ouder dan 90 dagen uit de FTS5-index om de schijfbelasting compact te houden.
* **`PRAGMA optimize` & `VACUUM`:** Wekelijks om 04:00 's nachts draait de daemon automatisch `PRAGMA optimize;` en `VACUUM;` op SQLite om fragmentatie te elimineren.
* **Migratie Integriteit:** Database-migraties worden bij opstarten automatisch gevalideerd en toegepast via `sqlx::migrate!()` met ingebouwde checksums.

---

## 9. Rust Projectstructuur

```text
ircord/
├── Cargo.toml
├── .env.example
├── Dockerfile
├── docker-compose.yml
├── config.toml                # Multi-channel configuratie met live reload
├── migrations/
│   └── 20260908_init.sql      # Schema voor presence, WP, seen, afk, tell, feeds, audit & FTS5
├── scripts/                   # Map voor dynamische Rhai scripts
│   └── hallo.rhai
├── tests/                     # Integratietests & Mocks
│   ├── bridge_tests.rs
│   └── plugin_tests.rs
└── src/
    ├── main.rs                # Initialisatie, configuratie en orchestration
    ├── config.rs              # Struct validatie (Serde) en live reload watcher
    ├── bridge/
    │   ├── mod.rs             # Event definities (BridgeMessage)
    │   └── router.rs          # Multi-channel routing, LRU message cache & loop-guard
    ├── irc/
    │   ├── client.rs          # IRC connectie, TLS, SASL, presence & reconnect loop
    │   └── flood.rs           # Token-bucket rate limiter voor IRC
    ├── discord/
    │   ├── handler.rs         # Serenity event handlers (berichten, presence, DM's)
    │   └── webhook.rs         # Discord webhook dispatcher met 429 backoff
    ├── ai/
    │   ├── freetoken.rs       # FlashML FreeToken HTTP client & prompt generator
    │   ├── vision.rs          # VLM image summarizer voor IRC (met hash cache)
    │   ├── manager.rs         # Model switching policy & token budgeting
    │   └── rag.rs             # SQLite FTS5 context retrieval
    ├── web/
    │   ├── server.rs          # Axum HTTP listener voor GitHub webhooks & metrics
    │   └── github.rs          # GitHub webhook payload parser met HMAC-validatie
    ├── plugins/               # === CLOUDBOT / IRCPLUS / AI / WP PLUGINS ===
    │   ├── mod.rs             # Plugin Trait, Lifecycle supervision & PluginManager
    │   ├── whatpulse.rs       # WhatPulse API client (Team de Apen op grandmasg.nl)
    │   ├── ai.rs              # FreeToken LLM Plugin (!ai, !tldr, !topic)
    │   ├── presence.rs        # !online, !lastonline, !seen en quit logging
    │   ├── afk.rs             # !afk en auto-notificaties bij mentions
    │   ├── seen_tell.rs       # Offline memo's (!tell)
    │   ├── karma.rs           # Karma tracker (nick++, nick--)
    │   ├── sed.rs             # Regex string replacement (s/oud/nieuw/)
    │   ├── url_titler.rs      # HTML / OpenGraph scraper
    │   ├── rss.rs             # Publieke & persoonlijke RSS reader (!rss, !track, !digest)
    │   ├── raid_guard.rs      # Anti-raid & clone flood shield
    │   ├── weather.rs         # Open-Meteo weer plugin (!weer)
    │   ├── slap.rs            # Trout slap met CTCP ACTION (!slap)
    │   ├── remind.rs          # Asynchrone Tokio reminders (!remindme)
    │   ├── quotes.rs          # Quotes database (!quote)
    │   ├── alias.rs           # Custom aliases manager (!alias)
    │   ├── poll.rs            # Hybride stempeilingen (!poll)
    │   ├── reactions.rs       # Discord reaction naar IRC bridge
    │   ├── topic.rs           # Kanaaltopic geschiedenis & suggesties
    │   ├── log_export.rs      # Chatlog zoeker en exporteerder (!log)
    │   ├── safety.rs          # Anti-phishing en URL safety guard
    │   ├── stats.rs           # Kanaalstatistieken (!top, !peak)
    │   ├── rhai_runner.rs     # Gesandboxte Rhai dynamic scripting bridge
    │   └── admin.rs           # Kanaalstatistieken, audit logging & botstatus (!status)
    └── utils/
        ├── sanitizer.rs       # Discord mentions resolving & mIRC color stripper
        ├── auth.rs            # NickServ account status check & RBAC validator
        ├── lifecycle.rs       # CancellationToken & Graceful Shutdown flow
        └── pastebin.rs        # Multiline codeblock uploader (0x0.st)
```

---

## 10. `Cargo.toml` Configuratie

```toml
[package]
name = "ircord"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1.40", features = ["full"] }
tokio-util = { version = "0.7", features = ["sync"] }
serenity = { version = "0.12", default-features = false, features = ["client", "gateway", "rustls_backend", "model", "cache"] }
irc = "1.0"
axum = "0.7"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "multipart"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.8"
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio-rustls", "sqlite", "macros", "migrate"] }
async-trait = "0.1"
regex = "1.10"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
dotenvy = "0.15"
scraper = "0.20"
rhai = { version = "1.19", features = ["sync"] }
chrono = { version = "0.4", features = ["serde"] }
feed-rs = "1.5"
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"
notify = "6.1"
lru = "0.12"

[dev-dependencies]
tokio-test = "0.4"
wiremock = "0.6"
```

---

## 11. Teststrategie & Kwaliteitsborging

Voor een betrouwbare 24/7 daemon implementeren we drie testlagen:
1. **Unit Tests (`cargo test --lib`):**
   * Verificatie van string sanitizers (mIRC codes strippen, zero-width space injectie, Discord mention omzettingen).
   * Token-bucket rate limiter logica (wachttijden en burst-limieten).
   * HMAC-verificatie voor GitHub webhooks.
2. **Plugin Tests via Mock Eventbus (`tests/plugin_tests.rs`):**
   * Aanroepen van plugins via een in-memory SQLite database (`sqlite::memory:`) en mock HTTP-client (`wiremock`).
   * Testen van `!weer`, `!seen`, `!tell` en `!wp` zonder externe netwerkafhankelijkheid.
3. **Bridge Loop & Deduplication Tests (`tests/bridge_tests.rs`):**
   * Simulatie van inkomende Discord webhooks om te controleren of de loop-guard ze 100% tijdig onderschept en weggooit.
   * Valideren van de LRU-cache bij native Discord replies.

---

## 12. Resource Profile & Performance Benchmarks

Dankzij de zero-overhead asynchrone Rust architectuur hanteert de daemon een uiterst voorspelbaar resourceprofiel:

| Scenario / Belasting | Verwacht RAM-gebruik | Verwachte CPU-belasting | Latency Impact |
| :--- | :--- | :--- | :--- |
| **Idle State (Verbonden met IRC & Discord)** | ~7 - 10 MB | 0.0% - 0.1% | N.v.t. |
| **Normale Chat (1-3 kanalen actief)** | ~11 - 14 MB | < 0.2% | < 3 ms per bridge event |
| **Hoge Belasting (10 gekoppelde kanalen)** | ~15 - 18 MB | < 0.5% | < 5 ms per bridge event |
| **Actieve AI Generatie (FreeToken streaming)** | ~16 - 20 MB | < 0.8% | Bepaald door FreeToken inferentiesnelheid |
| **VLM Afbeeldingsanalyse (Vision)** | ~18 - 22 MB | < 1.0% | CPU/GPU load ligt 100% in FreeToken proces |

*Opmerking: Zelfs bij zware pieken blijft de memory footprint gegarandeerd onder de 25 MB, waardoor de daemon moeiteloos kan draaien op de kleinste VPS of Raspberry Pi.*

---

## 13. Docker & Productie

### Multi-stage `Dockerfile` (< 25 MB footprint)
```dockerfile
# Build stage
FROM rust:1.80-alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig openssl-dev sqlite-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

# Runtime stage
FROM alpine:3.20
RUN apk add --no-cache ca-certificates tzdata sqlite-libs
WORKDIR /app
COPY --from=builder /app/target/release/ircord /app/ircord
COPY scripts ./scripts

EXPOSE 9090
ENTRYPOINT ["/app/ircord"]
```

### `docker-compose.yml`
```yaml
services:
  ircord:
    build: .
    container_name: ircord_core
    restart: unless-stopped
    ports:
      - "9090:9090"
    environment:
      - DISCORD_TOKEN=${DISCORD_TOKEN}
      - IRC_SERVER=irc.libera.chat
      - IRC_PORT=6697
      - IRC_NICK=IRCordBot
      - DATABASE_URL=sqlite:///app/data/ircord.db
      - FREETOKEN_BASE_URL=http://host.docker.internal:8000/v1
      - FREETOKEN_MODEL=default
      - WHATPULSE_API_URL=https://www.grandmasg.nl/WPNEW/api.php
      - GITHUB_WEBHOOK_SECRET=jouw_geheime_webhook_token
    extra_hosts:
      - "host.docker.internal:host-gateway"
    volumes:
      - ./data:/app/data
      - ./scripts:/app/scripts
      - ./config.toml:/app/config.toml
```

---

## 14. Fasering & Roadmap

| Fase | Focus | Concrete Acties |
| :--- | :--- | :--- |
| **Fase 1: Rust Core, Config & Safety** | Architectuur | `Plugin` trait en `PluginManager` met panic-isolatie, Serde config validatie, live reload, graceful shutdown en SQLite pool met checksum migraties. |
| **Fase 2: FlashML FreeToken AI Hub** | Intelligentie | `FreeTokenClient` bouwen voor `/v1/chat/completions`, model manager (`!setmodel`), token-budgettering en vision hash-cache. |
| **Fase 3: Multi-Channel Bridge & Loops** | Connectiviteit | Serenity Discord client, TLS IRC client met SASL, LRU cache voor replies, loop-guard, 429 webhook backoff en pastebin uploader. |
| **Fase 4: Community, Presence & WhatPulse** | Community | `plugin_whatpulse` koppelen aan `grandmasg.nl/WPNEW/` en officiële API, `!online`, `!lastonline` (met quit logging) en `!afk`. |
| **Fase 5: Feeds, HMAC Webhooks & Rhai** | Automatisering | Axum webhook listener met HMAC-SHA256, `!rss` (publiek + privé DM), gesandboxte Rhai engine en audit trail logging. |
| **Fase 6: Productie, Tests & Database Lifecycle** | Stabiliteit | Unit/integratietests draaien, automatische database retentie/VACUUM activeren, multi-stage Docker build en poort 9090 metrics valideren. |
