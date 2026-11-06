//! User settings: what each one means, validation, and the file they're kept in.
//!
//! Only settings that change real behaviour belong here. The active pick is not one of them: it
//! lives in WFP. Everything except `load` and `save` is pure.

use crate::diag;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// How a setting's value is edited.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Number,
    Toggle,
    Server,
    Path,
    Theme,
}

struct Key {
    name: &'static str,
    title: &'static str,
    kind: Kind,
    allowed: &'static str,
    about: &'static str,
}

const KEYS: &[Key] = &[
    Key {
        name: "backup",
        title: "Default backup site",
        kind: Kind::Server,
        allowed: "none or a server code",
        about: "Keeps a second site open from the next pick on, so the game can have a failover \
                relay. none means one relay with no failover.",
    },
    Key {
        name: "ping_every",
        title: "Direct ping interval (s)",
        kind: Kind::Number,
        allowed: "0 to 300",
        about: "How often dl-rs pings the pick for the Direct counter. 0 stops all ping traffic.",
    },
    Key {
        name: "refresh_minutes",
        title: "Relay refresh (minutes)",
        kind: Kind::Number,
        allowed: "0, or 5 to 1440",
        about: "How often the relay list is fetched from Steam. 0 is manual, which also stops \
                dl-rs repairing removed filters on its own.",
    },
    Key {
        name: "hold_in_game",
        title: "Hold filter updates in game",
        kind: Kind::Toggle,
        allowed: "on or off",
        about: "Delays background filter rewrites while a game runs, because each one makes \
                Windows re-check live connections. New relays stay unblocked until the game closes.",
    },
    Key {
        name: "report_every",
        title: "Console report interval (s)",
        kind: Kind::Number,
        allowed: "1 to 3600",
        about: "How often the console report repeats while it's running.",
    },
    Key {
        name: "report_on_open",
        title: "Report when console opens",
        kind: Kind::Toggle,
        allowed: "on or off",
        about: "Starts the report and the blocked traffic watcher when the console opens. Off \
                keeps the console quiet until you run start report.",
    },
    Key {
        name: "clear_on_exit",
        title: "Remove block on exit",
        kind: Kind::Toggle,
        allowed: "on or off",
        about: "Removes the block when dl-rs closes. Off leaves it on until reboot or the next pick.",
    },
    Key {
        name: "theme",
        title: "Theme",
        kind: Kind::Theme,
        allowed: "system, light or dark",
        about: "How the window looks. system follows your Windows setting.",
    },
    Key {
        name: "log_path",
        title: "Game log path",
        kind: Kind::Path,
        allowed: "auto or a file path",
        about: "The console.log for the report and the route counter. auto finds Deadlock's log. \
                Set a path to read another game's log.",
    },
];

/// One setting, as a UI or the console shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: &'static str,
    pub title: &'static str,
    pub kind: Kind,
    pub value: String,
    pub allowed: &'static str,
    pub about: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `None` means strict blocking: one relay and no failover.
    pub backup: Option<String>,
    pub ping_every: u32,
    pub refresh_minutes: u32,
    pub hold_in_game: bool,
    pub report_every: u32,
    pub report_on_open: bool,
    pub clear_on_exit: bool,
    /// `None` finds Deadlock's log automatically.
    pub log_path: Option<String>,
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            backup: None,
            ping_every: 5,
            refresh_minutes: 5,
            hold_in_game: false,
            report_every: 5,
            report_on_open: true,
            clear_on_exit: false,
            log_path: None,
            theme: "system".to_string(),
        }
    }
}

fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.to_string()
}

fn toggle(key: &str, value: &str) -> Result<bool, String> {
    match value.to_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{key} must be on or off, not '{value}'")),
    }
}

fn number(
    key: &str,
    value: &str,
    allowed: &str,
    valid: impl Fn(u32) -> bool,
) -> Result<u32, String> {
    value
        .parse()
        .ok()
        .filter(|number| valid(*number))
        .ok_or_else(|| format!("{key} must be {allowed}, not '{value}'"))
}

impl Settings {
    /// A setting's current value, in the form `set` accepts.
    fn value(&self, key: &str) -> String {
        match key {
            "backup" => self.backup.clone().unwrap_or_else(|| "none".to_string()),
            "ping_every" => self.ping_every.to_string(),
            "refresh_minutes" => self.refresh_minutes.to_string(),
            "hold_in_game" => on_off(self.hold_in_game),
            "report_every" => self.report_every.to_string(),
            "report_on_open" => on_off(self.report_on_open),
            "clear_on_exit" => on_off(self.clear_on_exit),
            "log_path" => self.log_path.clone().unwrap_or_else(|| "auto".to_string()),
            "theme" => self.theme.clone(),
            _ => String::new(),
        }
    }

    /// Validates and stores one value. On error nothing changes. Whether a backup names a real
    /// server depends on the relay list, so the session checks that.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        match key {
            "backup" => {
                let code = value.to_lowercase();
                if code.is_empty() || code.contains(char::is_whitespace) {
                    return Err(format!("backup must be none or a server, not '{value}'"));
                }
                self.backup = (code != "none").then_some(code);
            }
            "ping_every" => {
                self.ping_every = number(key, value, "0 to 300", |n| n <= 300)?;
            }
            "refresh_minutes" => {
                self.refresh_minutes = number(key, value, "0, or 5 to 1440", |n| {
                    n == 0 || (5..=1440).contains(&n)
                })?;
            }
            "hold_in_game" => self.hold_in_game = toggle(key, value)?,
            "report_every" => {
                self.report_every = number(key, value, "1 to 3600", |n| (1..=3600).contains(&n))?;
            }
            "report_on_open" => self.report_on_open = toggle(key, value)?,
            "clear_on_exit" => self.clear_on_exit = toggle(key, value)?,
            "theme" => {
                let theme = value.to_lowercase();
                if !["system", "light", "dark"].contains(&theme.as_str()) {
                    return Err(format!(
                        "theme must be system, light or dark, not '{value}'"
                    ));
                }
                self.theme = theme;
            }
            "log_path" => {
                let path = value.trim_matches('"');
                if path.is_empty() {
                    return Err("log_path must be auto or a file path".to_string());
                }
                self.log_path = (!path.eq_ignore_ascii_case("auto")).then(|| path.to_string());
            }
            _ => return Err(unknown(key)),
        }
        Ok(())
    }

    /// Puts one setting back to its default.
    pub fn reset(&mut self, key: &str) -> Result<(), String> {
        let default = Self::default().value(key);
        self.set(key, &default)
    }

    pub fn rows(&self) -> Vec<Row> {
        KEYS.iter()
            .map(|key| Row {
                key: key.name,
                title: key.title,
                kind: key.kind,
                value: self.value(key.name),
                allowed: key.allowed,
                about: key.about,
            })
            .collect()
    }

    /// Reads settings text. A bad line keeps that setting's default and adds a note, so a damaged
    /// file can't stop the app or change a block.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut settings = Self::default();
        let mut notes = Vec::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                notes.push(format!("settings: ignored '{line}'"));
                continue;
            };
            if let Err(error) = settings.set(key.trim(), value) {
                notes.push(format!("settings: {error}, using the default"));
            }
        }
        (settings, notes)
    }

    pub fn to_text(&self) -> String {
        let mut text = String::from("; dl-rs settings. Change these in the app with F2.\r\n");
        for key in KEYS {
            let _ = write!(text, "{}={}\r\n", key.name, self.value(key.name));
        }
        text
    }
}

fn unknown(key: &str) -> String {
    format!("unknown setting '{key}'. Try 'show config'")
}

pub fn is_key(key: &str) -> bool {
    KEYS.iter().any(|known| known.name == key)
}

/// Setting names and titles, for completion.
pub fn names() -> impl Iterator<Item = (&'static str, &'static str)> {
    KEYS.iter().map(|key| (key.name, key.title))
}

/// How a setting's value is edited, if the key exists.
pub fn kind(key: &str) -> Option<Kind> {
    KEYS.iter()
        .find(|known| known.name == key)
        .map(|known| known.kind)
}

/// Where settings are kept. `None` if Windows doesn't say, and then they last only this session.
pub fn default_path() -> Option<PathBuf> {
    diag::data_dir().map(|folder| folder.join("settings.ini"))
}

/// A missing file is normal and gives the defaults.
pub fn load(path: &Path) -> (Settings, Vec<String>) {
    match fs::read_to_string(path) {
        Ok(text) => Settings::parse(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (Settings::default(), Vec::new())
        }
        Err(error) => (
            Settings::default(),
            vec![format!(
                "settings: can't read the file, using defaults: {error}"
            )],
        ),
    }
}

/// Writes through a temporary file, so a crash can't leave half a settings file behind.
pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    let write = || -> std::io::Result<()> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let temporary = path.with_extension("ini.tmp");
        fs::write(&temporary, settings.to_text())?;
        fs::rename(&temporary, path)
    };
    write().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(key: &str, value: &str) -> Result<Settings, String> {
        let mut settings = Settings::default();
        settings.set(key, value).map(|()| settings)
    }

    #[test]
    fn defaults_match_what_the_app_did_before_settings_existed() {
        let settings = Settings::default();
        assert_eq!(settings.backup, None);
        assert_eq!(settings.refresh_minutes, 5);
        assert_eq!(settings.report_every, 5);
        assert!(settings.report_on_open);
        assert!(!settings.clear_on_exit);
        assert!(!settings.hold_in_game);
    }

    #[test]
    fn accepts_values_at_the_edges_of_each_range() {
        assert_eq!(set("ping_every", "0").unwrap().ping_every, 0);
        assert_eq!(set("ping_every", "300").unwrap().ping_every, 300);
        assert_eq!(set("refresh_minutes", "0").unwrap().refresh_minutes, 0);
        assert_eq!(set("refresh_minutes", "5").unwrap().refresh_minutes, 5);
        assert_eq!(
            set("refresh_minutes", "1440").unwrap().refresh_minutes,
            1440
        );
        assert_eq!(set("report_every", "1").unwrap().report_every, 1);
        assert_eq!(set("report_every", "3600").unwrap().report_every, 3600);
    }

    #[test]
    fn rejects_values_outside_each_range_with_a_clear_error() {
        for (key, value, expected) in [
            ("ping_every", "301", "0 to 300"),
            ("ping_every", "soon", "'soon'"),
            ("ping_every", "-1", "0 to 300"),
            ("refresh_minutes", "4", "0, or 5 to 1440"),
            ("refresh_minutes", "1441", "0, or 5 to 1440"),
            ("report_every", "0", "1 to 3600"),
            ("report_on_open", "maybe", "on or off"),
            ("hold_in_game", "1", "on or off"),
            ("theme", "blue", "system, light or dark"),
            ("clear_on_exit", "", "on or off"),
            ("backup", "two words", "none or a server"),
            ("log_path", "\"\"", "auto or a file path"),
            ("nope", "1", "unknown setting 'nope'"),
        ] {
            let error = set(key, value).unwrap_err();
            assert!(error.contains(expected), "{key}={value}: {error}");
        }
    }

    #[test]
    fn a_rejected_value_changes_nothing() {
        let mut settings = Settings::default();
        settings.set("ping_every", "10").unwrap();
        assert!(settings.set("ping_every", "999").is_err());
        assert_eq!(settings.ping_every, 10);
    }

    #[test]
    fn backup_and_log_path_have_a_none_form() {
        assert_eq!(
            set("backup", "MAA2").unwrap().backup.as_deref(),
            Some("maa2")
        );
        assert_eq!(set("backup", "none").unwrap().backup, None);
        assert_eq!(set("log_path", "AUTO").unwrap().log_path, None);
    }

    #[test]
    fn a_log_path_keeps_its_case_and_spaces() {
        let path = r#""D:\Steam Games\Game\console.log""#;
        assert_eq!(
            set("log_path", path).unwrap().log_path.as_deref(),
            Some(r"D:\Steam Games\Game\console.log")
        );
    }

    #[test]
    fn saved_text_reads_back_the_same() {
        let mut settings = Settings::default();
        settings.set("backup", "maa2").unwrap();
        settings.set("ping_every", "30").unwrap();
        settings.set("refresh_minutes", "0").unwrap();
        settings.set("hold_in_game", "on").unwrap();
        settings.set("report_on_open", "off").unwrap();
        settings.set("clear_on_exit", "on").unwrap();
        settings.set("log_path", r"D:\a b\console.log").unwrap();
        settings.set("theme", "dark").unwrap();
        let (read, notes) = Settings::parse(&settings.to_text());
        assert_eq!(read, settings);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_damaged_file_keeps_defaults_and_says_so() {
        let (read, notes) =
            Settings::parse("ping_every=banana\nnonsense\nreport_every=30\nmystery=1\n");
        assert_eq!(read.ping_every, 5);
        assert_eq!(read.report_every, 30);
        assert_eq!(notes.len(), 3, "{notes:?}");
    }

    #[test]
    fn reset_restores_the_default() {
        let mut settings = Settings::default();
        settings.set("ping_every", "60").unwrap();
        settings.set("backup", "maa2").unwrap();
        settings.reset("ping_every").unwrap();
        settings.reset("backup").unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn every_key_has_a_row_with_its_explanation() {
        let rows = Settings::default().rows();
        assert_eq!(rows.len(), KEYS.len());
        for row in rows {
            assert!(!row.value.is_empty(), "{}", row.key);
            assert!(!row.about.is_empty(), "{}", row.key);
            assert!(!row.title.is_empty(), "{}", row.key);
        }
    }

    #[test]
    fn load_and_save_round_trip_through_a_real_file() {
        let folder = std::env::temp_dir().join(format!("dl-rs-test-{}", std::process::id()));
        let path = folder.join("nested").join("settings.ini");
        let (missing, notes) = load(&path);
        assert_eq!(missing, Settings::default());
        assert!(notes.is_empty());
        let mut settings = Settings::default();
        settings.set("ping_every", "42").unwrap();
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).0, settings);
        fs::remove_dir_all(&folder).unwrap();
    }
}
