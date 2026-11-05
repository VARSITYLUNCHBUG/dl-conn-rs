//! The command language: `verb noun [options]`.
//!
//! One table drives parsing, help and completion, so they can't disagree. Nothing here touches
//! Windows or app state.

/// A relay site's code and name, as offered by completion.
pub type ServerName = (String, String);

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help { topic: Option<String> },
    ListServers { filter: Option<String> },
    ListRelays { server: Option<String> },
    ShowStatus,
    SetPick { server: String },
    ClearPick,
    RefreshRelays,
    ShowRoutes { server: Option<String> },
    PingRelays { server: Option<String> },
    ShowProcesses,
    ShowReport,
    StartReport { every: Option<u64> },
    StopReport,
    ToggleConsole,
    ClearScreen,
    Quit,
}

enum Param {
    Server {
        required: bool,
    },
    Topic,
    Option {
        name: &'static str,
        value: &'static str,
        about: &'static str,
    },
}

#[derive(Default)]
struct Args {
    server: Option<String>,
    topic: Option<String>,
    options: Vec<(&'static str, String)>,
}

impl Args {
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
        params: &[Param::Server { required: true }],
        summary: "Block every relay except this server's",
        build: |args| {
            Ok(Command::SetPick {
                server: args.server.unwrap_or_default(),
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
        summary: "Show or hide this console, and its report with it",
        build: |_| Ok(Command::ToggleConsole),
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
                Param::Option { name, value, .. } => format!("[{name} {value}]"),
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
                Param::Option { name, value, about } => {
                    lines.push(format!("  {name} {value}  {about}"));
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
        if let Some((name, value)) = option {
            let Some(given) = rest.next() else {
                return Err(format!("{name} needs {value}. Usage: {}", spec.usage()));
            };
            args.options.push((name, given.clone()));
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
    (spec.build)(args)
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
        lines.push("Tab completes. ? explains what you've typed. ` hides the console.".to_string());
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
    // A value is expected right after an option, and there's nothing useful to suggest for it.
    let awaiting_value = given.last().is_some_and(|last| {
        spec.params
            .iter()
            .any(|param| matches!(param, Param::Option { name, .. } if name == last))
    });
    if awaiting_value {
        return Vec::new();
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
            Param::Server { .. } if !word.starts_with('-') && !has_server(given) => {
                candidates.extend(matching_servers(word, servers));
            }
            _ => {}
        }
    }
    candidates
}

fn has_server(given: &[String]) -> bool {
    given.iter().any(|arg| !arg.starts_with("--"))
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
            if let Param::Server { required: true } = param {
                line.push_str(" fra");
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
                server: "fra".to_string()
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
        assert_eq!(texts("help set "), ["pick"]);
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
        assert!(texts("set pick fra ").is_empty());
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
        assert_eq!(partial.len(), 4, "{partial:?}");
        assert!(help_for("wibble").len() >= COMMANDS.len());
    }

    #[test]
    fn help_rejects_unknown_topics() {
        assert!(help(Some("wibble")).is_err());
        assert_eq!(help(Some("show")).unwrap().len(), 4);
    }
}
