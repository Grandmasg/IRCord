use super::{MessageEvent, Plugin, PluginContext};
use async_trait::async_trait;
use regex::RegexBuilder;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Aantal berichten per kanaal waarin we terugzoeken (zoals Limnoria SedRegex / sed-bots).
const HISTORY_PER_CHANNEL: usize = 50;
const MAX_PATTERN_LEN: usize = 100;
const MAX_OUTPUT_CHARS: usize = 400;

/// Een geparseerd `s/patroon/vervanging/vlaggen` commando.
#[derive(Debug, PartialEq)]
struct SedCommand {
    /// `nick: s/a/b/` corrigeert het bericht van een ander.
    target: Option<String>,
    pattern: String,
    replacement: String,
    global: bool,
    ignore_case: bool,
    /// `s/a/b/2` vervangt alleen het 2e voorkomen.
    nth: Option<usize>,
}

/// Splitst op een niet-ge-escapete delimiter; `\<delim>` wordt de letterlijke delimiter.
fn split_unescaped(input: &str, delim: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some(&n) if n == delim => {
                    parts.last_mut().unwrap().push(n);
                    chars.next();
                }
                Some(&n) => {
                    let cur = parts.last_mut().unwrap();
                    cur.push('\\');
                    cur.push(n);
                    chars.next();
                }
                None => parts.last_mut().unwrap().push('\\'),
            }
        } else if c == delim {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    parts
}

fn parse_sed(content: &str) -> Option<SedCommand> {
    let mut rest = content.trim();

    // Optioneel "nick: " of "nick, " voorvoegsel
    let mut target = None;
    if let Some(idx) = rest.find([':', ',']) {
        let (nick, after) = rest.split_at(idx);
        let after = after[1..].trim_start();
        if !nick.is_empty()
            && !nick.contains(char::is_whitespace)
            && nick.len() <= 32
            && after.starts_with('s')
            && after.len() > after.trim_start_matches('s').len()
        {
            target = Some(nick.to_string());
            rest = after;
        }
    }

    let mut it = rest.chars();
    if it.next()? != 's' {
        return None;
    }
    let delim = it.next()?;
    if delim.is_alphanumeric() || delim.is_whitespace() || delim == '\\' {
        return None;
    }
    let body: String = it.collect();
    let parts = split_unescaped(&body, delim);
    // patroon, vervanging [, vlaggen]
    if parts.len() < 2 || parts.len() > 3 || parts[0].is_empty() || parts[0].len() > MAX_PATTERN_LEN {
        return None;
    }
    let flags = parts.get(2).map(String::as_str).unwrap_or("");
    let mut cmd = SedCommand {
        target,
        pattern: parts[0].clone(),
        replacement: parts[1].clone(),
        global: false,
        ignore_case: false,
        nth: None,
    };
    let mut digits = String::new();
    for f in flags.chars() {
        match f {
            'g' | 'G' => cmd.global = true,
            'i' | 'I' => cmd.ignore_case = true,
            d if d.is_ascii_digit() => digits.push(d),
            _ => return None,
        }
    }
    if !digits.is_empty() {
        cmd.nth = digits.parse().ok().filter(|n: &usize| *n >= 1);
    }
    Some(cmd)
}

/// Zet sed-vervangingen (`\1`, `&`) om naar regex-crate syntax (`${1}`, `${0}`); `$` blijft letterlijk.
fn convert_replacement(repl: &str) -> String {
    let mut out = String::new();
    let mut chars = repl.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(d) if d.is_ascii_digit() => out.push_str(&format!("${{{}}}", d)),
                Some('n') => out.push(' '),
                Some(o) => out.push(o),
                None => out.push('\\'),
            },
            '&' => out.push_str("${0}"),
            '$' => out.push_str("$$"),
            o => out.push(o),
        }
    }
    out
}

/// Past het commando toe op één bericht; `None` als het patroon niet voorkomt of niets verandert.
fn apply(cmd: &SedCommand, text: &str) -> Option<String> {
    let build = |p: &str| RegexBuilder::new(p).case_insensitive(cmd.ignore_case).size_limit(1 << 20).build();
    // Ongeldige regex (bijv. "s/(foo/bar/") valt terug op letterlijke tekst.
    let re = build(&cmd.pattern).or_else(|_| build(&regex::escape(&cmd.pattern))).ok()?;
    if !re.is_match(text) {
        return None;
    }
    let repl = convert_replacement(&cmd.replacement);
    let out = if let Some(n) = cmd.nth {
        let m = re.find_iter(text).nth(n - 1)?;
        let mut s = String::with_capacity(text.len());
        s.push_str(&text[..m.start()]);
        let caps = re.captures_at(text, m.start())?;
        caps.expand(&repl, &mut s);
        s.push_str(&text[m.end()..]);
        s
    } else if cmd.global {
        re.replace_all(text, repl.as_str()).into_owned()
    } else {
        re.replace(text, repl.as_str()).into_owned()
    };
    (out != text).then_some(out)
}

pub struct SedPlugin {
    /// Kanaal -> laatste berichten (nieuwste achteraan)
    history: Mutex<HashMap<String, VecDeque<(String, String)>>>,
}

impl SedPlugin {
    pub fn new() -> Self {
        Self { history: Mutex::new(HashMap::new()) }
    }
}

#[async_trait]
impl Plugin for SedPlugin {
    fn name(&self) -> &'static str { "sed" }
    fn help(&self) -> &'static str {
        "s/oud/nieuw/[gi] - Corrigeert je vorige bericht | nick: s/oud/nieuw/ - corrigeert dat van een ander"
    }

    async fn on_message(&self, ctx: &PluginContext, msg: &MessageEvent) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trimmed = msg.content.trim();
        if msg.author.eq_ignore_ascii_case("IRCord") || msg.author.eq_ignore_ascii_case("Monkeybot") {
            return Ok(None);
        }
        let key = format!("{}/{}", msg.platform, msg.channel.to_lowercase());

        if let Some(cmd) = parse_sed(trimmed) {
            let wanted = cmd.target.as_deref().unwrap_or(&msg.author);
            let history = self.history.lock().unwrap();
            // Zoek van nieuw naar oud het eerste bericht van de gewenste nick waar het patroon in voorkomt.
            let found = history.get(&key).and_then(|h| {
                h.iter()
                    .rev()
                    .filter(|(nick, _)| nick.eq_ignore_ascii_case(wanted))
                    .find_map(|(nick, text)| apply(&cmd, text).map(|c| (nick.clone(), c)))
            });
            if let Some((nick, corrected)) = found {
                let corrected: String = corrected.chars().take(MAX_OUTPUT_CHARS).collect();
                return Ok(Some(if cmd.target.is_some() && !nick.eq_ignore_ascii_case(&msg.author) {
                    format!("✏️ {} denkt dat {} bedoelde: {}", msg.author, nick, corrected)
                } else {
                    format!("✏️ {} bedoelde: {}", nick, corrected)
                }));
            }
            return Ok(None);
        }

        if !trimmed.is_empty() && !ctx.config.general.is_command_trigger(trimmed) {
            let mut history = self.history.lock().unwrap();
            let h = history.entry(key).or_default();
            h.push_back((msg.author.clone(), msg.content.clone()));
            if h.len() > HISTORY_PER_CHANNEL {
                h.pop_front();
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cmd: &str, text: &str) -> Option<String> {
        apply(&parse_sed(cmd).expect("parse"), text)
    }

    #[test]
    fn basic_and_flags() {
        assert_eq!(run("s/mij/ik", "beter dan mij").as_deref(), Some("beter dan ik"));
        assert_eq!(run("s/a/b/", "aaa").as_deref(), Some("baa"));
        assert_eq!(run("s/a/b/g", "aaa").as_deref(), Some("bbb"));
        assert_eq!(run("s/A/b/i", "aaa").as_deref(), Some("baa"));
        assert_eq!(run("s/a/b/2", "aaa").as_deref(), Some("aba"));
        assert_eq!(run("s/x/y/", "aaa"), None);
    }

    #[test]
    fn regex_groups_and_delimiters() {
        assert_eq!(run(r"s/(\w+) (\w+)/\2 \1/", "hallo wereld").as_deref(), Some("wereld hallo"));
        assert_eq!(run("s#/usr#/opt#", "pad /usr/bin").as_deref(), Some("pad /opt/bin"));
        assert_eq!(run(r"s/a\/b/c/", "a/b").as_deref(), Some("c"));
        assert_eq!(run("s/(foo/bar/", "x (foo y").as_deref(), Some("x bar y"));
        assert_eq!(run("s/prijs/$5/", "de prijs").as_deref(), Some("de $5"));
    }

    #[test]
    fn target_nick_and_rejects() {
        let c = parse_sed("PjoT: s/a/b/").unwrap();
        assert_eq!(c.target.as_deref(), Some("PjoT"));
        assert!(parse_sed("sinterklaas komt: s/a/b/").is_none());
        assert!(parse_sed("s/a/b/x").is_none());
        assert!(parse_sed("sorry dat is mooi").is_none());
        assert!(parse_sed("s//b/").is_none());
    }
}
