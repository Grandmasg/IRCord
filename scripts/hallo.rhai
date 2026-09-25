// Voorbeeld Rhai script voor IRCord
// Kan live worden uitgevoerd of geladen worden zonder de daemon te herstarten.

let user = params[0];
if user == "" {
    user = "bezoeker";
}

let begroetingen = [
    "Van harte welkom in het kanaal, " + user + "!",
    "Hoi " + user + ", fijn dat je er bent!",
    "Gegroet " + user + "! Typ !help voor een overzicht van commando's."
];

let index = rand() % 3;
begroetingen[index]
