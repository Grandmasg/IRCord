# IRCord — Installatie- en Deploymenthandleiding 🚀

Deze handleiding biedt stapsgewijze instructies voor het installeren, configureren en operationeel beheren van de **IRCord** hybride IRC-Discord bot daemon in productie.

---

## 📋 Inhoudsopgave
1. [Systeemvereisten](#1-systeemvereisten)
2. [Discord Bot & Webhooks Configureren](#2-discord-bot--webhooks-configureren)
3. [IRC & SASL Account Configureren](#3-irc--sasl-account-configureren)
4. [Configuratiebestanden (.env & config.toml)](#4-configuratiebestanden-env--configtoml)
5. [Installatie Optie A: Docker Compose (Aanbevolen)](#5-installatie-optie-a-docker-compose-aanbevolen)
6. [Installatie Optie B: Bare-Metal / Native Rust](#6-installatie-optie-b-bare-metal--native-rust)
7. [Verificatie & Testen](#7-verificatie--testen)
8. [Probleemoplossing & Veelgestelde Vragen (FAQ)](#8-probleemoplossing--veelgestelde-vragen-faq)

---

## 1. Systeemvereisten

### Voor de IRCord Daemon
Geschreven in Rust is IRCord uitzonderlijk lichtgewicht en zuinig:
- **Werkgeheugen (RAM):** < 20 MB RAM actief runtime-gebruik.
- **CPU:** 1 virtuele core (0.25 - 0.5 vCPU is ruimschoots voldoende).
- **Schijfruimte:** ~50 MB voor de binary, SQLite database en logs.
- **Besturingssysteem:** Linux (Debian, Ubuntu, Alpine, Arch), macOS of Windows.

### Voor FlashML FreeToken AI (Optioneel, bij lokale hosting)
- **RAM:** Minimaal 8 GB RAM (16–32 GB aanbevolen voor 7B/14B MoE modellen).
- **GPU (Optioneel):** AMD ROCm (Radeon 780M/890M/RX 7000+) of NVIDIA CUDA. Co-executie via de CPU wordt ondersteund.
*Opmerking: Als je geen lokaal LLM wilt draaien, kun je het AI-endpoint verwijzen naar een willekeurige OpenAI-compatibele server of de functie uitgeschakeld laten.*

---

## 2. Discord Bot & Webhooks Configureren

Om de bridge tussen IRC en Discord tot stand te brengen, zijn een **Discord Bot Token** en **Discord Webhooks** vereist.

### Stap 2.1: Discord Applicatie & Bot Aanmaken
1. Ga naar het [Discord Developer Portal](https://discord.com/developers/applications) en log in.
2. Klik rechtsboven op **New Application** en geef een naam op (bijv. `IRCord`).
3. Klik in het linkermenu op **Bot**.
4. Klik op **Reset Token** (of *Add Bot*) en kopieer het gegenereerde token. Dit is jouw `DISCORD_BOT_TOKEN`.
5. Scroll naar beneden naar **Privileged Gateway Intents** en schakel het volgende in:
   - ✅ **MESSAGE CONTENT INTENT** *(Essentieel: zonder deze intent kan de bot geen kanaalberichten lezen)*
   - ✅ **SERVER MEMBERS INTENT** *(Aanbevolen voor presence- en gebruikerssynchronisatie)*
6. Klik op **Save Changes**.

### Stap 2.2: Nodig de Bot Uit op Jouw Discord Server
1. Ga in het linkermenu naar **OAuth2 ➔ URL Generator**.
2. Vink onder **Scopes** aan: `bot`.
3. Vink onder **Bot Permissions** aan:
   - `Send Messages`
   - `Manage Webhooks`
   - `Read Message History`
   - `Embed Links`
   - `Attach Files`
4. Kopieer de gegenereerde URL onderaan en open deze in je browser om de bot te autoriseren en toe te voegen aan je Discord server.

### Stap 2.3: Discord Webhooks Aanmaken
Voor elk Discord-kanaal dat gekoppeld wordt aan IRC gebruikt de bot een webhook om IRC-berichten door te sturen met de nick en avatar van de IRC-gebruiker:
1. Open Discord, klik met de rechtermuisknop op het gewenste tekstkanaal en kies **Edit Channel** (tandwiel-icoon).
2. Ga naar **Integraties ➔ Webhooks ➔ Nieuwe Webhook**.
3. Geef de webhook een naam (bijv. `IRCord Relay`).
4. Klik op **Webhook-URL kopiëren**. Bewaar deze URL voor de koppeling in `config.toml`.

---

## 3. IRC & SASL Account Configureren

Veel moderne IRC-netwerken (zoals Libera.Chat, OFTC of Ergo) vereisen SASL-authenticatie om direct tijdens de TLS-handshake te authenticeren voordat kanalen worden gejoined. Dit voorkomt 'nick in use' conflicten en biedt toegang tot kanalen die alleen geregistreerde nicks toelaten (`+r`).

1. Registreer de nick van je bot bij NickServ op je IRC-netwerk:
   ```text
   /msg NickServ REGISTER <wachtwoord> <e-mailadres>
   ```
2. Noteer de accountgebruikersnaam en het wachtwoord. Deze komen overeen met `IRC_SASL_USER` en `IRC_SASL_PASS`.

---

## 4. Configuratiebestanden (.env & config.toml)

### Stap 4.1: `.env` Configureren
Kopieer het voorbeeldbestand voor omgevingsvariabelen:
```bash
cp .env.example .env
# Of gebruik de Nederlandse of Duitse toelichting:
# cp .env.nl.example .env
# cp .env.de.example .env
```

Open `.env` in je teksteditor en vul de waarden in:
```dotenv
# Discord Inloggegevens
DISCORD_BOT_TOKEN=MTE5OT...jouw_echte_token_hier...
GITHUB_WEBHOOK_SECRET=jouw_geheime_webhook_sleutel

# IRC Verbindingsinstellingen
IRC_SERVER=irc.libera.chat
IRC_PORT=6697
IRC_NICK=IRCordBot
IRC_USER=ircord
IRC_REALNAME=IRCord Hybrid AI Daemon
IRC_PASSWORD=

# IRCv3 SASL Authenticatie
IRC_SASL_USER=IRCordBot
IRC_SASL_PASS=jouw_geheime_sasl_wachtwoord

# SQLite Database
DATABASE_URL=sqlite:ircord.db

# FlashML FreeToken Lokaal Endpoint & Model
FREETOKEN_BASE_URL=http://127.0.0.1:1919/v1
FREETOKEN_MODEL=default
FREETOKEN_API_KEY=
```

### Stap 4.2: `config.toml` Configureren
Kopieer het configuratie voorbeeldbestand:
```bash
cp config.example.nl.toml config.toml
# Of gebruik de Engelse of Duitse template:
# cp config.example.toml config.toml
# cp config.example.de.toml config.toml
```
*(Tip: Je kunt tijdens runtime ook een aangepast configuratiepad opgeven via de `CONFIG_PATH` omgevingsvariabele, bijv. `CONFIG_PATH=config.prod.toml`.)*

Open `config.toml` en stel algemene voorkeuren, commando-voorvoegsels, kanaalkoppelingen en talen in:
```toml
[general]
language = "nl" # Standaard taal: "nl", "en", "de", "fr", "es"
command_prefixes = ["!", "."] # Herkende commando-voorvoegsels (bijv. !help, .help)
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

# Koppel hier jouw kanalen
[[channels]]
irc_channel = "#algemeen"
discord_channel_id = 123456789012345678
discord_webhook_url = "https://discord.com/api/webhooks/..."
# Optionele kanaaltaal (Gebruikersvoorkeur '!lang' > Kanaaltaal > Algemene [general].language)
# language = "nl"

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

## 5. Installatie Optie A: Docker Compose (Aanbevolen) 🐳

IRCord levert **3 overzichtelijke, specifieke Docker Compose bestanden** mee, plus een interactieve launcher zodat je eenvoudig de gewenste stack start zonder ingewikkelde parameters:

### 0. Snelle Interactieve Launcher (Eenvoudigste optie)
- **Windows**: Dubbelklik of voer uit: [`start.bat`](start.bat)
- **Linux / NAS**: Voer uit: `./start.sh`

---

### 1. Ollama ROCm + GPU Setup (Aanbevolen voor Minisforum N5 Pro & Radeon 890M)
*Ideaal voor Minisforum N5 Pro, AMD Ryzen AI / Radeon iGPU, of servers die een alles-in-één stack willen.*
```bash
docker compose -f docker-compose.ollama.yml up -d --build
# of simpelweg 'docker compose up -d --build'
```
* **Ollama (ROCm GPU)** start automatisch op.
* **`ollama-init`** downloadt automatisch het geconfigureerde `FREETOKEN_MODEL` (bijv. `qwen2.5:7b`).
* **`ircord`** start en verbindt direct met Ollama.

### 2. FlashML FreeToken Engine Setup
*Voor het draaien van de dedicated FlashML FreeToken ROCm 6.1 container.*
```bash
docker compose -f docker-compose.freetoken.yml up -d --build
```

### 3. Standalone IRCord Bot (Externe AI op Host of LAN)
*Wanneer je reeds Ollama, vLLM of LM Studio draait op je host OS of een aparte server.*
```bash
docker compose -f docker-compose.standalone.yml up -d --build
```

### Stap 5.2: Status & Model Download Verifiëren
```bash
# Controleer draaiende containers
docker compose ps

# Bekijk live IRCord logs
docker compose logs -f ircord

# Controleer downloadvoortgang van het AI model (bij het Ollama profiel)
docker compose logs -f ircord-ollama-init
```

### Stap 5.3: Containers Beheren
- Stack stoppen: `docker compose down`
- Daemon herstarten: `docker compose restart ircord`
- Wisselen van AI model: pas `FREETOKEN_MODEL` in `.env` aan en herstart de stack.

---

## 6. Installatie Optie B: Bare-Metal / Native Rust

Als je IRCord rechtstreeks op je host of VPS wilt uitvoeren:

### Stap 6.1: Vereisten installeren
Installeer de Rust toolchain (Rust 1.75+ aanbevolen):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

Installeer SQLite libraries en build-tools:
```bash
# Debian / Ubuntu
sudo apt-get update && sudo apt-get install -y libsqlite3-dev pkg-config libssl-dev build-essential
```

### Stap 6.2: De Release Binary Compileren
```bash
cargo build --release
```
De geoptimaliseerde binary wordt aangemaakt in `target/release/ircord`.

### Stap 6.3: Systemd Service (Linux Daemon)
Maak een systemd unit bestand aan in `/etc/systemd/system/ircord.service`:
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

Activeer en start de service:
```bash
sudo systemctl daemon-reload
sudo systemctl enable ircord
sudo systemctl start ircord
sudo journalctl -u ircord -f
```

---

## 7. Verificatie & Testen

1. **Web API Healthcheck controleren:**
   ```bash
   curl http://localhost:9090/health
   # Verwacht antwoord: {"status":"healthy"}
   ```

2. **Diagnostische buffer controleren:**
   ```bash
   curl http://localhost:9090/api/errors
   ```

3. **In-Chat Rooktest:**
   - Op IRC of Discord: Typ `!ping` (de bot antwoordt met `pong!`).
   - Typ `!status` (de bot toont uptime, actieve plugins en geheugengebruik).
   - Typ `!weer Amsterdam` of `!weather Amsterdam`.
   - Typ `!crypto btc`.
   - Plaats een YouTube link om metadata- en omschrijvingsparsing te controleren.

---

## 8. Probleemoplossing & Veelgestelde Vragen (FAQ)

### Probleem: De bot joint IRC, maar ontvangt geen Discord-berichten
- **Oorzaak:** De Privileged Gateway Intent ontbreekt in het Discord Developer Portal.
- **Oplossing:** Ga naar Discord Developer Portal ➔ Application ➔ Bot ➔ Schakel **MESSAGE CONTENT INTENT** in.

### Probleem: De bot kan geen `+r` kanalen joinen op IRC
- **Oorzaak:** SASL inloggegevens ontbreken of zijn onjuist.
- **Oplossing:** Controleer `IRC_SASL_USER` en `IRC_SASL_PASS` in `.env`. Zorg dat je nick geregistreerd is bij NickServ.

### Probleem: Foutmeldingen overspoelen openbare chatkanalen
- **Ontwerp:** Volledige diagnostiek (`!errors`) is beperkt tot privéberichten (PM) of het aangewezen admin-logkanaal (`#bot-logs` op IRC, configureerbaar Discord kanaal-ID). Publieke kanalen ontvangen enkel een korte verwijzing.
