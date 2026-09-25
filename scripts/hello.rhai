// Example Rhai script for IRCord
// Hot-reloadable: can be executed or modified live without restarting the daemon.

let user = params[0];
if user == "" {
    user = "visitor";
}

let greetings = [
    "Welcome to the channel, " + user + "!",
    "Hey " + user + ", glad to see you here!",
    "Greetings " + user + "! Type !help for an overview of available commands."
];

let index = rand() % 3;
greetings[index]
