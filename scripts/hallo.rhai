// Taalafhankelijk Rhai script voor IRCord (Nederlandse standaard)
// Hot-reloadable: begroet standaard in het Nederlands, maar respecteert ook andere talen.

let user = if params.len() > 0 && params[0] != "" {
    params[0]
} else if is_def_var("author") && author != "" {
    author
} else {
    "bezoeker"
};

// Check taal: meegegeven via argument of bot-omgeving (standaard "nl")
let current_lang = if params.len() > 1 && params[1] != "" {
    params[1]
} else if is_def_var("lang") && lang != "" {
    lang
} else {
    "nl"
};

let begroetingen = [];

if current_lang == "nl" {
    begroetingen = [
        "Welkom in het kanaal, " + user + "!",
        "Hoi " + user + ", leuk je hier te zien!",
        "Gegroet " + user + "! Typ !help voor een overzicht van beschikbare commando's."
    ];
} else if current_lang == "de" {
    begroetingen = [
        "Willkommen im Kanal, " + user + "!",
        "Hallo " + user + ", schön dich hier zu sehen!",
        "Grüße " + user + "! Tippe !help für eine Übersicht der verfügbaren Befehle."
    ];
} else {
    begroetingen = [
        "Welcome to the channel, " + user + "!",
        "Hey " + user + ", glad to see you here!",
        "Greetings " + user + "! Type !help for an overview of available commands."
    ];
}

let index = rand() % begroetingen.len();
if index < 0 { index = -index; }
begroetingen[index]
