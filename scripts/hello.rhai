// Multilingual / language-dependent Rhai greeting script for IRCord
// Hot-reloadable: automatically adapts to the bot's configured language (lang),
// or accepts an explicit language override via argument (e.g. !script hello Alice nl).

let user = if params.len() > 0 && params[0] != "" {
    params[0]
} else if is_def_var("author") && author != "" {
    author
} else {
    "visitor"
};

// Check language: passed in params, injected from bot config/locale (lang), or defaults to English
let current_lang = if params.len() > 1 && params[1] != "" {
    params[1]
} else if is_def_var("lang") && lang != "" {
    lang
} else {
    "en"
};

let greetings = [];

if current_lang == "nl" {
    let nl_user = if user == "visitor" { "bezoeker" } else { user };
    greetings = [
        "Welkom in het kanaal, " + nl_user + "!",
        "Hoi " + nl_user + ", leuk je hier te zien!",
        "Gegroet " + nl_user + "! Typ !help voor een overzicht van beschikbare commando's."
    ];
} else if current_lang == "de" {
    let de_user = if user == "visitor" { "Besucher" } else { user };
    greetings = [
        "Willkommen im Kanal, " + de_user + "!",
        "Hallo " + de_user + ", schön dich hier zu sehen!",
        "Grüße " + de_user + "! Tippe !help für eine Übersicht der verfügbaren Befehle."
    ];
} else {
    greetings = [
        "Welcome to the channel, " + user + "!",
        "Hey " + user + ", glad to see you here!",
        "Greetings " + user + "! Type !help for an overview of available commands."
    ];
}

let index = rand() % greetings.len();
if index < 0 { index = -index; }
greetings[index]
