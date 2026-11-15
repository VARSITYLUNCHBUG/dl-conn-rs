//! The command language: `verb noun [options]`.
//!
//! One table drives parsing, help and completion, so they can't disagree. Nothing here touches
//! Windows or app state.

use crate::settings;

/// A relay site's code and name, as offered by completion.
pub type ServerName = (String, String);

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help {
        topic: Option<String>,
    },
    ListServers {
        filter: Option<String>,
    },
    ListRelays {
        server: Option<String>,
    },
    ShowStatus,
    SetPick {
        server: String,
        backup: Option<String>,
        /// Switch even if that would cut the match being played.
        now: bool,
    },
    ClearPick,
    ClearPending,
    RefreshRelays,
    ShowRoutes {
        server: Option<String>,
    },
    PingRelays {
        server: Option<String>,
    },
    ShowProcesses,
    ShowReport,
    StartReport {
        every: Option<u64>,
    },
    StopReport,
    ToggleConsole,
    ClearScreen,
    ShowConfig,
    SetConfig {
        key: String,
        value: String,
    },
    ClearConfig {
        key: Option<String>,
    },
    AnalyzeLog,
    ShowGuide,
    Quit,
}

enum Param {
    Server {
        required: bool,
    },
    Topic,
    /// A setting's name.
    Key {
        required: bool,
    },
    /// Everything after the key, exactly as typed, so a path keeps its case and spaces.
    Value,
    Option {
        name: &'static str,
        value: &'static str,
        about: &'static str,
    },
    /// An option that takes no value.
    Flag {
        name: &'static str,
        about: &'static str,
    },
}

#[derive(Default)]
struct Args {
    server: Option<String>,
    topic: Option<String>,
    key: Option<String>,
    value: Option<String>,
    options: Vec<(&'static str, String)>,
    flags: Vec<&'static str>,
}

impl Args {
    fn flag(&self, name: &str) -> bool {
        self.flags.contains(&name)
    }

    fn option(&self, name: &str) -> Option<String> {
        self.options
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    }
}

struct Spec {
    verb: &'static str,
    /// Empty for one-word commands such as `quit`.
    noun: &'static str,
    params: &'static [Param],
    summary: &'static str,
    build: fn(Args) -> Result<Command, String>,
}

const OPTIONAL_SERVER: &[Param] = &[Param::Server { required: false }];
/// An option with this value completes to server codes.
const SERVER_VALUE: &str = "<server>";

static COMMANDS: &[Spec] = &[
    Spec {
        verb: "help",
        noun: "",
        params: &[Param::Topic],
        summary: "List commands, or explain one",
        build: |args| Ok(Command::Help { topic: args.topic }),
    },
    Spec {
        verb: "list",
        noun: "servers",
        params: &[Param::Option {
            name: "--filter",
            value: "<text>",
            about: "Only servers whose code or name contains this",
        }],
        summary: "List relay sites you can pick",
        build: |args| {
            Ok(Command::ListServers {
                filter: args.option("--filter"),
            })
        },
    },
    Spec {
        verb: "list",
        noun: "relays",
        params: OPTIONAL_SERVER,
        summary: "List relay addresses, for all sites or one",
        build: |args| {
            Ok(Command::ListRelays {
                server: args.server,
            })
        },
    },
    Spec {
        verb: "show",
        noun: "status",
        params: &[],
        summary: "Show the pick, what's blocked and connection state",
        build: |_| Ok(Command::ShowStatus),
    },
    Spec {
        verb: "set",
        noun: "pick",
        params: &[
            Param::Server { required: true },
            Param::Option {
                name: "--backup",
                value: SERVER_VALUE,
                about: "Also keep this server open for failover. none skips the default backup",
            },
            Param::Flag {
                name: "--now",
                about: "Switch now even if it cuts the match you're in",
            },
        ],
        summary: "Block every relay except this server's. Mid-match, waits for the match to end",
        build: |args| {
            let backup = args.option("--backup");
            let now = args.flag("--now");
            let server = args.server.unwrap_or_default();
            if backup.as_ref() == Some(&server) {
                return Err("--backup must be a different server from the pick".to_string());
            }
            Ok(Command::SetPick {
                server,
                backup,
                now,
            })
        },
    },
    Spec {
        verb: "clear",
        noun: "pick",
        params: &[],
        summary: "Remove the block so all servers are open",
        build: |_| Ok(Command::ClearPick),
    },
    Spec {
        verb: "clear",
        noun: "pending",
        params: &[],
        summary: "Drop a pick that's waiting for the match to end",
        build: |_| Ok(Command::ClearPending),
    },
    Spec {
        verb: "clear",
        noun: "screen",
        params: &[],
        summary: "Clear the console",
        build: |_| Ok(Command::ClearScreen),
    },
    Spec {
        verb: "refresh",
        noun: "relays",
        params: &[],
        summary: "Fetch relay lists from Steam now",
        build: |_| Ok(Command::RefreshRelays),
    },
    Spec {
        verb: "show",
        noun: "routes",
        params: OPTIONAL_SERVER,
        summary: "Show the interface and gateway to each relay",
        build: |args| {
            Ok(Command::ShowRoutes {
                server: args.server,
            })
        },
    },
    Spec {
        verb: "ping",
        noun: "relays",
        params: OPTIONAL_SERVER,
        summary: "Measure latency to each relay",
        build: |args| {
            Ok(Command::PingRelays {
                server: args.server,
            })
        },
    },
    Spec {
        verb: "show",
        noun: "processes",
        params: &[],
        summary: "Show running game, Steam and ExitLag processes",
        build: |_| Ok(Command::ShowProcesses),
    },
    Spec {
        verb: "show",
        noun: "report",
        params: &[],
        summary: "Print processes, pick, relays and blocked traffic once",
        build: |_| Ok(Command::ShowReport),
    },
    Spec {
        verb: "start",
        noun: "report",
        params: &[Param::Option {
            name: "--every",
            value: "<secs>",
            about: "Seconds between reports, from 1 to 3600 (default 5)",
        }],
        summary: "Repeat the report and collect blocked traffic",
        build: |args| {
            let every = match args.option("--every") {
                None => None,
                Some(text) => match text.parse::<u64>() {
                    Ok(secs @ 1..=3600) => Some(secs),
                    _ => return Err(format!("--every must be 1 to 3600 seconds, not '{text}'")),
                },
            };
            Ok(Command::StartReport { every })
        },
    },
    Spec {
        verb: "stop",
        noun: "report",
        params: &[],
        summary: "Stop repeating the report",
        build: |_| Ok(Command::StopReport),
    },
    Spec {
        verb: "toggle",
        noun: "console",
        params: &[],
        summary: "Open or close the console page, and its report with it",
        build: |_| Ok(Command::ToggleConsole),
    },
    Spec {
        verb: "show",
        noun: "config",
        params: &[],
        summary: "Show every setting and what it does",
        build: |_| Ok(Command::ShowConfig),
    },
    Spec {
        verb: "set",
        noun: "config",
        params: &[Param::Key { required: true }, Param::Value],
        summary: "Change a setting and save it",
        build: |args| {
            Ok(Command::SetConfig {
                key: args.key.unwrap_or_default(),
                value: args.value.unwrap_or_default(),
            })
        },
    },
    Spec {
        verb: "clear",
        noun: "config",
        params: &[Param::Key { required: false }],
        summary: "Put one setting, or all of them, back to the default",
        build: |args| Ok(Command::ClearConfig { key: args.key }),
    },
    Spec {
        verb: "analyze",
        noun: "log",
        params: &[],
        summary: "Read the game's log and report on its relay connections",
        build: |_| Ok(Command::AnalyzeLog),
    },
    Spec {
        verb: "show",
        noun: "guide",
        params: &[],
        summary: "Show how to test a connection, step by step",
        build: |_| Ok(Command::ShowGuide),
    },
    Spec {
        verb: "quit",
        noun: "",
        params: &[],
        summary: "Close the app. Blocks stay on until reboot or the next pick",
        build: |_| Ok(Command::Quit),
    },
];

impl Spec {
    fn name(&self) -> String {
        if self.noun.is_empty() {
            self.verb.to_string()
        } else {
            format!("{} {}", self.verb, self.noun)
        }
    }

    fn usage(&self) -> String {
        let mut usage = self.name();
        for param in self.params {
            usage.push(' ');
            usage.push_str(&match param {
                Param::Server { required: true } => "<server>".to_string(),
                Param::Server { required: false } => "[<server>]".to_string(),
                Param::Topic => "[verb [noun]]".to_string(),
                Param::Key { required: true } => "<key>".to_string(),
                Param::Key { required: false } => "[<key>]".to_string(),
                Param::Value => "<value>".to_string(),
                Param::Option { name, value, .. } => format!("[{name} {value}]"),
                Param::Flag { name, .. } => format!("[{name}]"),
            });
        }
        usage
    }

    fn details(&self) -> Vec<String> {
        let mut lines = vec![self.usage(), format!("  {}", self.summary)];
        for param in self.params {
            match param {
                Param::Server { required: true } => {
                    lines.push("  <server>  A code from 'list servers'".to_string());
                }
                Param::Server { required: false } => {
                    lines.push(
                        "  <server>  A code from 'list servers' (default: the pick)".to_string(),
                    );
                }
                Param::Topic => {}
                Param::Key { .. } => {
                    lines.push("  <key>  A setting from 'show config'".to_string());
                }
                Param::Value => {
                    lines.push("  <value>  'show config' lists what each one allows".to_string());
                }
                Param::Option { name, value, about } => {
                    lines.push(format!("  {name} {value}  {about}"));
                }
                Param::Flag { name, about } => {
                    lines.push(format!("  {name}  {about}"));
                }
            }
        }
        lines
    }
}

/// The spec a line names, and how many words its name used up.
fn find(words: &[String]) -> Option<(&'static Spec, usize)> {
    let verb = words.first()?;
    COMMANDS.iter().find_map(|spec| {
        if spec.verb != verb {
            None
        } else if spec.noun.is_empty() {
            Some((spec, 1))
        } else if words.get(1).is_some_and(|noun| noun == spec.noun) {
            Some((spec, 2))
        } else {
            None
        }
    })
}

fn is_verb(word: &str) -> bool {
    COMMANDS.iter().any(|spec| spec.verb == word)
}

fn lower_words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_lowercase).collect()
}

pub fn parse(line: &str) -> Result<Command, String> {
    let words = lower_words(line);
    if words.is_empty() {
        return Err("type a command, or 'help'".to_string());
    }
    let Some((spec, used)) = find(&words) else {
        return Err(unknown_command(&words));
    };
    let mut args = Args::default();
    let takes_value = spec
        .params
        .iter()
        .any(|param| matches!(param, Param::Value));
    let takes_key = spec
        .params
        .iter()
        .any(|param| matches!(param, Param::Key { .. }));
    let mut rest = words[used..].iter();
    if spec
        .params
        .iter()
        .any(|param| matches!(param, Param::Topic))
    {
        let topic = words[used..].join(" ");
        args.topic = (!topic.is_empty()).then_some(topic);
        rest = [].iter();
    }
    while let Some(word) = rest.next() {
        let option = spec.params.iter().find_map(|param| match param {
            Param::Option { name, value, .. } if name == word => Some((*name, *value)),
            _ => None,
        });
        let takes_server = spec
            .params
            .iter()
            .any(|param| matches!(param, Param::Server { .. }));
        let flag = spec.params.iter().find_map(|param| match param {
            Param::Flag { name, .. } if name == word => Some(*name),
            _ => None,
        });
        if let Some((name, value)) = option {
            let Some(given) = rest.next() else {
                return Err(format!("{name} needs {value}. Usage: {}", spec.usage()));
            };
            args.options.push((name, given.clone()));
        } else if let Some(name) = flag {
            args.flags.push(name);
        } else if takes_key && args.key.is_none() {
            if !settings::is_key(word) {
                return Err(format!("unknown setting '{word}'. Try 'show config'"));
            }
            args.key = Some(word.clone());
            if takes_value {
                let value = after_words(line, used + 1);
                args.value = (!value.is_empty()).then(|| value.to_string());
                break;
            }
        } else if takes_server && args.server.is_none() && !word.starts_with("--") {
            args.server = Some(word.clone());
        } else {
            return Err(format!("unexpected '{word}'. Usage: {}", spec.usage()));
        }
    }
    let needs_server = spec
        .params
        .iter()
        .any(|param| matches!(param, Param::Server { required: true }));
    if needs_server && args.server.is_none() {
        return Err(format!("missing <server>. Usage: {}", spec.usage()));
    }
    let needs_key = spec
        .params
        .iter()
        .any(|param| matches!(param, Param::Key { required: true }));
    if needs_key && args.key.is_none() {
        return Err(format!("missing <key>. Usage: {}", spec.usage()));
    }
    if takes_value && args.value.is_none() {
        return Err(format!("missing <value>. Usage: {}", spec.usage()));
    }
    (spec.build)(args)
}

/// The text after the first `count` words, as typed.
fn after_words(line: &str, count: usize) -> &str {
    let mut rest = line.trim();
    for _ in 0..count {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        rest = rest[end..].trim_start();
    }
    rest
}

fn unknown_command(words: &[String]) -> String {
    let verb = &words[0];
    if is_verb(verb) {
        let usages: Vec<String> = COMMANDS
            .iter()
            .filter(|spec| spec.verb == verb)
            .map(Spec::usage)
            .collect();
        return match words.get(1) {
            Some(noun) => format!("can't {verb} '{noun}'. Try: {}", usages.join(" | ")),
            None => format!("{verb} what? Try: {}", usages.join(" | ")),
        };
    }
    let typed = words.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
    match closest(&typed) {
        Some(name) => format!("unknown command '{typed}'. Did you mean '{name}'?"),
        None => format!("unknown command '{typed}'. Try 'help'"),
    }
}

/// The command name nearest to what was typed, if any is close enough to be a typo.
fn closest(typed: &str) -> Option<String> {
    const MAX_TYPOS: usize = 3;
    COMMANDS
        .iter()
        .map(Spec::name)
        .map(|name| (edit_distance(typed, &name), name))
        .filter(|(distance, _)| *distance <= MAX_TYPOS)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, name)| name)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.bytes().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right) in b.bytes().enumerate() {
            let cost = diagonal + usize::from(left != right);
            diagonal = row[j + 1];
            row[j + 1] = cost.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

/// The answer to `help` or `help <topic>`.
pub fn help(topic: Option<&str>) -> Result<Vec<String>, String> {
    let Some(topic) = topic else {
        let mut lines = table(COMMANDS.iter());
        lines.push(String::new());
        lines
            .push("Tab completes. ? explains what you've typed. ` closes the console.".to_string());
        return Ok(lines);
    };
    let words = lower_words(topic);
    if let Some((spec, _)) = find(&words) {
        return Ok(spec.details());
    }
    if words.first().is_some_and(|verb| is_verb(verb)) && words.len() == 1 {
        return Ok(table(COMMANDS.iter().filter(|spec| spec.verb == words[0])));
    }
    Err(unknown_command(&words))
}

/// Help for a half-typed line, for the `?` key. Always finds something useful to show.
pub fn help_for(line: &str) -> Vec<String> {
    let words = lower_words(line);
    if let Some((spec, _)) = find(&words) {
        return spec.details();
    }
    let typed = words.join(" ");
    let matches: Vec<&Spec> = COMMANDS
        .iter()
        .filter(|spec| spec.name().starts_with(&typed))
        .collect();
    if matches.is_empty() {
        table(COMMANDS.iter())
    } else {
        table(matches.into_iter())
    }
}

fn table<'a>(specs: impl Iterator<Item = &'a Spec>) -> Vec<String> {
    let rows: Vec<(String, &str)> = specs.map(|spec| (spec.usage(), spec.summary)).collect();
    let width = rows.iter().map(|(usage, _)| usage.len()).max().unwrap_or(0);
    rows.into_iter()
        .map(|(usage, summary)| format!("{usage:<width$}  {summary}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub text: String,
    pub note: String,
}

/// What can replace the text from `start` up to the cursor.
#[derive(Debug, PartialEq)]
pub struct Completion {
    pub start: usize,
    pub candidates: Vec<Candidate>,
}

/// Completes the word ending at `cursor`, a byte offset into `line`.
pub fn complete(line: &str, cursor: usize, servers: &[ServerName]) -> Completion {
    let before = &line[..cursor];
    let start = before.rfind(char::is_whitespace).map_or(0, |at| at + 1);
    let word = before[start..].to_lowercase();
    let previous = lower_words(&before[..start]);
    let mut candidates = match previous.as_slice() {
        [] => verbs(&word),
        [help] if help == "help" => verbs(&word),
        [help, verb] if help == "help" => nouns(verb, &word),
        [verb] if !COMMANDS.iter().any(|s| s.verb == verb && s.noun.is_empty()) => {
            nouns(verb, &word)
        }
        _ => arguments(&previous, &word, servers),
    };
    candidates.sort_by(|a, b| a.text.cmp(&b.text));
    Completion { start, candidates }
}

fn verbs(word: &str) -> Vec<Candidate> {
    let mut names: Vec<&str> = COMMANDS
        .iter()
        .map(|spec| spec.verb)
        .filter(|verb| verb.starts_with(word))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|verb| {
            let forms: Vec<&Spec> = COMMANDS.iter().filter(|spec| spec.verb == verb).collect();
            let note = match forms.as_slice() {
                [only] => only.summary.to_string(),
                many => many
                    .iter()
                    .map(|spec| spec.noun)
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            Candidate {
                text: verb.to_string(),
                note,
            }
        })
        .collect()
}

fn nouns(verb: &str, word: &str) -> Vec<Candidate> {
    COMMANDS
        .iter()
        .filter(|spec| spec.verb == verb && !spec.noun.is_empty() && spec.noun.starts_with(word))
        .map(|spec| Candidate {
            text: spec.noun.to_string(),
            note: spec.summary.to_string(),
        })
        .collect()
}

fn arguments(previous: &[String], word: &str, servers: &[ServerName]) -> Vec<Candidate> {
    let Some((spec, used)) = find(previous) else {
        return Vec::new();
    };
    let given = &previous[used..];
    // Right after an option comes its value, and only some values have anything to suggest.
    let awaited = given.last().and_then(|last| {
        spec.params.iter().find_map(|param| match param {
            Param::Option { name, value, .. } if name == last => Some(*value),
            _ => None,
        })
    });
    if let Some(value) = awaited {
        return if value == SERVER_VALUE {
            matching_servers(word, servers)
        } else {
            Vec::new()
        };
    }
    let mut candidates = Vec::new();
    for param in spec.params {
        match param {
            Param::Option { name, value, about }
                if name.starts_with(word) && !given.iter().any(|arg| arg == name) =>
            {
                candidates.push(Candidate {
                    text: (*name).to_string(),
                    note: format!("{value}  {about}"),
                });
            }
            Param::Flag { name, about }
                if name.starts_with(word) && !given.iter().any(|arg| arg == name) =>
            {
                candidates.push(Candidate {
                    text: (*name).to_string(),
                    note: (*about).to_string(),
                });
            }
            Param::Server { .. } if !word.starts_with('-') && !has_server(spec, given) => {
                candidates.extend(matching_servers(word, servers));
            }
            Param::Key { .. } if given.is_empty() => {
                candidates.extend(
                    settings::names()
                        .filter(|(name, _)| name.starts_with(word))
                        .map(|(name, title)| Candidate {
                            text: name.to_string(),
                            note: title.to_string(),
                        }),
                );
            }
            Param::Value if given.len() == 1 => {
                candidates.extend(values_for(&given[0], word, servers));
            }
            _ => {}
        }
    }
    candidates
}

/// What a setting can be set to, where there's a short list.
fn values_for(key: &str, word: &str, servers: &[ServerName]) -> Vec<Candidate> {
    let fixed = |values: &[&str]| -> Vec<Candidate> {
        values
            .iter()
            .filter(|value| value.starts_with(word))
            .map(|value| Candidate {
                text: (*value).to_string(),
                note: String::new(),
            })
            .collect()
    };
    match settings::kind(key) {
        Some(settings::Kind::Toggle) => fixed(&["off", "on"]),
        Some(settings::Kind::Path) => fixed(&["auto"]),
        Some(settings::Kind::Theme) => fixed(&["dark", "light", "system"]),
        Some(settings::Kind::Server) => {
            let mut values = fixed(&["none"]);
            values.extend(matching_servers(word, servers));
            values
        }
        _ => Vec::new(),
    }
}

/// Whether the line already names the server, as opposed to an option's value.
fn has_server(spec: &Spec, given: &[String]) -> bool {
    let mut args = given.iter();
    while let Some(arg) = args.next() {
        let is_option = spec
            .params
            .iter()
            .any(|param| matches!(param, Param::Option { name, .. } if name == arg));
        if is_option {
            args.next();
        } else if !arg.starts_with("--") {
            return true;
        }
    }
    false
}

/// Servers whose code starts with the word, or, failing that, whose name contains it.
fn matching_servers(word: &str, servers: &[ServerName]) -> Vec<Candidate> {
    let candidate = |(code, desc): &ServerName| Candidate {
        text: code.clone(),
        note: desc.clone(),
    };
    let by_code: Vec<Candidate> = servers
        .iter()
        .filter(|(code, _)| code.starts_with(word))
        .map(candidate)
        .collect();
    if !by_code.is_empty() {
        return by_code;
    }
    servers
        .iter()
        .filter(|(_, desc)| desc.to_lowercase().contains(word))
        .map(candidate)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn servers() -> Vec<ServerName> {
        [
            ("fra", "Frankfurt (Germany)"),
            ("dfra", "Datapacket Frankfurt (Germany)"),
            ("jnb", "Johannesburg (South Africa)"),
            ("sto", "Stockholm - Kista (Sweden)"),
            ("sto2", "Stockholm - Bromma (Sweden)"),
        ]
        .iter()
        .map(|(code, desc)| ((*code).to_string(), (*desc).to_string()))
        .collect()
    }

    fn texts(line: &str) -> Vec<String> {
        complete(line, line.len(), &servers())
            .candidates
            .into_iter()
            .map(|candidate| candidate.text)
            .collect()
    }

    /// A valid invocation for each spec, built only from the table.
    fn example(spec: &Spec) -> String {
        let mut line = spec.name();
        for param in spec.params {
            match param {
                Param::Server { required: true } => line.push_str(" fra"),
                Param::Key { required: true } => line.push_str(" ping_every"),
                Param::Value => line.push_str(" 5"),
                _ => {}
            }
        }
        line
    }

    #[test]
    fn every_command_parses_from_its_own_name() {
        for spec in COMMANDS {
            let line = example(spec);
            assert!(parse(&line).is_ok(), "{line} should parse");
        }
    }

    #[test]
    fn every_command_appears_in_help_and_has_details() {
        let listing = help(None).unwrap().join("\n");
        for spec in COMMANDS {
            assert!(listing.contains(&spec.usage()), "{} missing", spec.name());
            let details = help(Some(&spec.name())).unwrap().join("\n");
            assert!(details.contains(spec.summary), "{} details", spec.name());
        }
    }

    #[test]
    fn parses_arguments_and_options() {
        assert_eq!(
            parse("set pick FRA"),
            Ok(Command::SetPick {
                server: "fra".to_string(),
                backup: None,
                now: false,
            })
        );
        assert_eq!(parse("  clear   pick "), Ok(Command::ClearPick));
        assert_eq!(
            parse("list relays"),
            Ok(Command::ListRelays { server: None })
        );
        assert_eq!(
            parse("list relays jnb"),
            Ok(Command::ListRelays {
                server: Some("jnb".to_string())
            })
        );
        assert_eq!(
            parse("list servers --filter south"),
            Ok(Command::ListServers {
                filter: Some("south".to_string())
            })
        );
        assert_eq!(
            parse("start report --every 30"),
            Ok(Command::StartReport { every: Some(30) })
        );
        assert_eq!(
            parse("start report"),
            Ok(Command::StartReport { every: None })
        );
        assert_eq!(parse("help"), Ok(Command::Help { topic: None }));
        assert_eq!(
            parse("help set pick"),
            Ok(Command::Help {
                topic: Some("set pick".to_string())
            })
        );
    }

    #[test]
    fn parses_a_backup_in_either_position() {
        let expected = Ok(Command::SetPick {
            server: "sgp".to_string(),
            backup: Some("maa2".to_string()),
            now: false,
        });
        assert_eq!(parse("set pick sgp --backup maa2"), expected);
        assert_eq!(parse("set pick --backup maa2 sgp"), expected);
        assert!(
            parse("set pick --backup maa2")
                .unwrap_err()
                .contains("missing <server>")
        );
        assert!(
            parse("set pick sgp --backup")
                .unwrap_err()
                .contains("needs <server>")
        );
        assert!(
            parse("set pick sgp --backup SGP")
                .unwrap_err()
                .contains("different server")
        );
    }

    #[test]
    fn now_is_a_flag_in_any_position() {
        let expected = Ok(Command::SetPick {
            server: "sgp".to_string(),
            backup: Some("maa2".to_string()),
            now: true,
        });
        assert_eq!(parse("set pick sgp --now --backup maa2"), expected);
        assert_eq!(parse("set pick --now sgp --backup maa2"), expected);
        assert_eq!(parse("set pick sgp --backup maa2 --now"), expected);
        assert_eq!(parse("clear pending"), Ok(Command::ClearPending));
        assert!(
            parse("clear pick --now")
                .unwrap_err()
                .contains("unexpected '--now'")
        );
        assert!(help(Some("set pick")).unwrap().join("\n").contains("--now"));
    }

    #[test]
    fn config_commands_keep_a_value_exactly_as_typed() {
        assert_eq!(
            parse("set config ping_every 30"),
            Ok(Command::SetConfig {
                key: "ping_every".to_string(),
                value: "30".to_string()
            })
        );
        assert_eq!(
            parse(r"  SET  Config   LOG_PATH   D:\Steam Games\console.log  "),
            Ok(Command::SetConfig {
                key: "log_path".to_string(),
                value: r"D:\Steam Games\console.log".to_string()
            })
        );
        assert_eq!(parse("show config"), Ok(Command::ShowConfig));
        assert_eq!(
            parse("clear config"),
            Ok(Command::ClearConfig { key: None })
        );
        assert_eq!(
            parse("clear config backup"),
            Ok(Command::ClearConfig {
                key: Some("backup".to_string())
            })
        );
    }

    #[test]
    fn config_commands_explain_what_is_wrong() {
        assert!(parse("set config").unwrap_err().contains("missing <key>"));
        assert!(
            parse("set config backup")
                .unwrap_err()
                .contains("missing <value>")
        );
        assert!(
            parse("set config nope 1")
                .unwrap_err()
                .contains("unknown setting 'nope'")
        );
        assert!(
            parse("clear config nope")
                .unwrap_err()
                .contains("unknown setting 'nope'")
        );
        assert!(
            parse("clear config backup extra")
                .unwrap_err()
                .contains("unexpected 'extra'")
        );
    }

    #[test]
    fn completes_config_keys_and_values() {
        assert_eq!(texts("set config ping"), ["ping_every"]);
        assert_eq!(
            texts("clear config rep"),
            ["report_every", "report_on_open"]
        );
        assert_eq!(texts("set config hold_in_game "), ["off", "on"]);
        assert_eq!(texts("set config clear_on_exit o"), ["off", "on"]);
        assert_eq!(texts("set config log_path "), ["auto"]);
        assert_eq!(texts("set config theme "), ["dark", "light", "system"]);
        assert_eq!(texts("set config backup j"), ["jnb"]);
        assert_eq!(texts("set config backup no"), ["none"]);
        assert!(texts("set config ping_every ").is_empty());
        assert!(texts("set config ping_every 5 ").is_empty());
    }

    #[test]
    fn parses_the_report_and_guide_commands() {
        assert_eq!(parse("analyze log"), Ok(Command::AnalyzeLog));
        assert_eq!(parse("show guide"), Ok(Command::ShowGuide));
        assert!(parse("analyze").unwrap_err().contains("analyze log"));
    }

    #[test]
    fn completes_a_backup_server() {
        assert_eq!(texts("set pick fra --backup j"), ["jnb"]);
        assert_eq!(texts("set pick fra --backup south"), ["jnb"]);
        assert_eq!(texts("set pick fra --b"), ["--backup"]);
        assert_eq!(texts("set pick fra "), ["--backup", "--now"]);
        assert_eq!(texts("set pick fra --now "), ["--backup"]);
        assert_eq!(texts("set pick fra --backup jnb "), ["--now"]);
        assert_eq!(texts("set pick fra --n"), ["--now"]);
        assert_eq!(texts("set pick --backup jnb f"), ["fra"]);
    }

    #[test]
    fn errors_explain_what_to_do() {
        assert!(parse("").unwrap_err().contains("help"));
        assert!(parse("set pick").unwrap_err().contains("set pick <server>"));
        assert!(parse("set").unwrap_err().contains("set pick <server>"));
        assert!(
            parse("set banana")
                .unwrap_err()
                .contains("can't set 'banana'")
        );
        assert!(
            parse("set pick fra jnb")
                .unwrap_err()
                .contains("unexpected 'jnb'")
        );
        assert!(
            parse("clear pick now")
                .unwrap_err()
                .contains("unexpected 'now'")
        );
        assert!(
            parse("start report --every")
                .unwrap_err()
                .contains("needs <secs>")
        );
        assert!(
            parse("start report --every soon")
                .unwrap_err()
                .contains("'soon'")
        );
        assert!(
            parse("start report --every 0")
                .unwrap_err()
                .contains("1 to 3600")
        );
        assert!(
            parse("list servers --nope")
                .unwrap_err()
                .contains("unexpected '--nope'")
        );
    }

    #[test]
    fn typos_get_a_suggestion() {
        assert!(parse("lst servers").unwrap_err().contains("'list servers'"));
        assert!(parse("shw status").unwrap_err().contains("'show status'"));
        assert!(
            parse("wibble wobble wubble")
                .unwrap_err()
                .contains("Try 'help'")
        );
    }

    #[test]
    fn completes_verbs_then_nouns() {
        assert_eq!(texts("li"), ["list"]);
        assert_eq!(texts("s"), ["set", "show", "start", "stop"]);
        assert_eq!(texts("list "), ["relays", "servers"]);
        assert_eq!(texts("list se"), ["servers"]);
        assert_eq!(texts("show p"), ["processes"]);
        assert!(texts("nope ").is_empty());
    }

    #[test]
    fn completes_help_topics() {
        assert_eq!(texts("help se"), ["set"]);
        assert_eq!(texts("help set "), ["config", "pick"]);
    }

    #[test]
    fn completes_servers_by_code_then_by_name() {
        assert_eq!(texts("set pick f"), ["fra"]);
        assert_eq!(texts("set pick sto"), ["sto", "sto2"]);
        assert_eq!(texts("set pick frank"), ["dfra", "fra"]);
        assert_eq!(texts("set pick south"), ["jnb"]);
        assert_eq!(texts("set pick nowhere"), Vec::<String>::new());
    }

    #[test]
    fn completes_options_and_stops_at_values() {
        assert_eq!(texts("start report "), ["--every"]);
        assert_eq!(texts("start report --e"), ["--every"]);
        assert!(texts("start report --every ").is_empty());
        assert!(texts("start report --every 5 ").is_empty());
        assert_eq!(texts("list servers --"), ["--filter"]);
    }

    #[test]
    fn stops_offering_a_server_once_one_is_given() {
        assert!(!texts("set pick fra ").contains(&"jnb".to_string()));
        assert!(texts("quit ").is_empty());
        assert!(texts("clear pick ").is_empty());
    }

    #[test]
    fn completes_at_the_cursor_not_the_end_of_the_line() {
        let completion = complete("li servers", 2, &servers());
        assert_eq!(completion.start, 0);
        assert_eq!(completion.candidates[0].text, "list");
    }

    #[test]
    fn completion_reports_where_the_word_starts() {
        assert_eq!(complete("set pick fr", 11, &servers()).start, 9);
    }

    #[test]
    fn help_for_explains_partial_lines() {
        assert!(help_for("set pick").join("\n").contains("<server>"));
        assert!(help_for("start report --ev").join("\n").contains("--every"));
        let partial = help_for("show");
        assert_eq!(partial.len(), 6, "{partial:?}");
        assert!(help_for("wibble").len() >= COMMANDS.len());
    }

    #[test]
    fn help_rejects_unknown_topics() {
        assert!(help(Some("wibble")).is_err());
        assert_eq!(help(Some("show")).unwrap().len(), 6);
    }
}
