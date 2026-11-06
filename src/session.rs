//! The app's core: owns the firewall, relays and pick, and runs commands one at a time.
//!
//! Front-ends only send command lines to a [`Handle`]. The session knows nothing about windows; a
//! [`Host`] gives it the few things it needs from whichever front-end started it.

use crate::analysis;
use crate::command::{self, Command, ServerName};
use crate::diag;
use crate::guide;
use crate::log;
use crate::monitor::{self, Control, Direct, Monitor};
use crate::sdr::{self, Blocks, Client, Pops};
use crate::settings::{self, Row, Settings};
use crate::wfp::Engine;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Failed fetches back off up to this, whatever the refresh setting.
const RETRY_MAX: Duration = Duration::from_secs(300);
/// Stands in for "never" when refreshing is manual.
const MANUAL: Duration = Duration::from_secs(365 * 24 * 3600);
/// Picking pulls the next fetch forward, but never closer than this to the last one.
const MIN_FETCH_GAP: Duration = Duration::from_secs(30);
const RETRY_MIN: Duration = Duration::from_secs(5);
const MAX_JITTER_MS: u32 = 15_000;

/// What the session blocks with. The real one is the WFP [`Engine`].
pub trait Firewall: Send + 'static {
    fn stamp(&self) -> Option<String>;
    fn apply(&self, stamp: &str, blocks: &Blocks) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
    fn watch_drops(&self, on: bool) -> Result<(), String>;
}

impl Firewall for Engine {
    fn stamp(&self) -> Option<String> {
        Engine::stamp(self)
    }
    fn apply(&self, stamp: &str, blocks: &Blocks) -> Result<(), String> {
        Engine::apply(self, stamp, blocks)
    }
    fn clear(&self) -> Result<(), String> {
        Engine::clear(self)
    }
    fn watch_drops(&self, on: bool) -> Result<(), String> {
        Engine::watch_drops(self, on)
    }
}

/// What a command needs from the front-end that started the session.
pub trait Host: Send + 'static {
    /// Shows or hides the console, returning whether it's now visible.
    fn toggle_console(&self) -> bool;
    fn quit(&self);
    /// State a front-end might display has changed, so it should query again.
    fn changed(&self);
}

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub code: String,
    pub desc: String,
    pub relays: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    /// Empty when nothing is picked.
    pub pick: String,
    /// `None` until the pick is found in a loaded relay list.
    pub pick_desc: Option<String>,
    pub backup: Option<String>,
    pub blocked: usize,
    pub sites_blocked: usize,
    pub loaded: bool,
    pub offline: bool,
    pub error: Option<String>,
    pub monitor: Monitor,
}

/// A command's result as data, so a UI never has to parse text.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Lines(Vec<String>),
    Servers(Vec<Server>),
    Status(Status),
    Config(Vec<Row>),
}

impl Output {
    fn line(text: String) -> Self {
        Self::Lines(vec![text])
    }

    /// The same result as log lines.
    pub fn render(&self) -> Vec<String> {
        match self {
            Self::Lines(lines) => lines.clone(),
            Self::Servers(servers) => {
                let mut rows: Vec<String> = servers
                    .iter()
                    .map(|server| {
                        let relays = server.relays;
                        format!("{:<6} {relays:>3} relays  {}", server.code, server.desc)
                    })
                    .collect();
                if rows.is_empty() {
                    rows.push("no servers".to_string());
                }
                rows
            }
            Self::Status(status) => {
                let pick = match (status.pick.as_str(), &status.pick_desc) {
                    ("", _) => "pick: none, all servers open".to_string(),
                    (pick, Some(desc)) => format!(
                        "pick: {pick} {desc}{}, {} relays blocked across {} sites",
                        status
                            .backup
                            .as_ref()
                            .map_or(String::new(), |backup| format!(", backup {backup}")),
                        status.blocked,
                        status.sites_blocked
                    ),
                    (pick, None) => format!("pick: {pick}, not in the loaded relay list"),
                };
                let mut lines = vec![pick];
                if !status.loaded {
                    lines.push("relays: not loaded yet".to_string());
                }
                if status.offline {
                    lines.push("network: offline, retrying".to_string());
                }
                if let Some(error) = &status.error {
                    lines.push(format!("error: {error}"));
                }
                lines.push(monitor::direct_label(&status.pick, &status.monitor));
                lines.push(monitor::route_label(&status.pick, &status.monitor));
                lines
            }
            Self::Config(rows) => rows
                .iter()
                .flat_map(|row| {
                    [
                        format!("{:<16} {}  ({})", row.key, row.value, row.allowed),
                        format!("    {}", row.about),
                    ]
                })
                .collect(),
        }
    }
}

pub type Reply = Result<Output, String>;

enum Message {
    Request {
        line: String,
        silent: bool,
        reply: Sender<Reply>,
    },
    Relays(Pops),
    Offline,
    Monitor(Monitor),
}

enum Wake {
    /// Refresh soon, respecting the minimum gap between fetches.
    Soon,
    Now,
    /// The refresh setting changed. In minutes, with 0 meaning manual.
    Interval(u32),
}

/// How front-ends reach the session. Cheap to clone.
#[derive(Clone)]
pub struct Handle {
    tx: Sender<Message>,
    servers: Arc<Mutex<Vec<ServerName>>>,
}

impl Handle {
    /// Runs a command as the user would: echoed to the log along with its result.
    pub fn run(&self, line: &str) -> Receiver<Reply> {
        self.send(line, false)
    }

    /// Runs a command for its data, without logging. For a UI keeping itself up to date.
    pub fn query(&self, line: &str) -> Receiver<Reply> {
        self.send(line, true)
    }

    /// Hands the session a relay list, as a fetch would.
    #[cfg(test)]
    pub fn inject_relays(&self, pops: Pops) {
        let _ = self.tx.send(Message::Relays(pops));
    }

    /// The known servers for completion. Available immediately, even during a slow command.
    pub fn servers(&self) -> Vec<ServerName> {
        self.servers.lock().unwrap().clone()
    }

    fn send(&self, line: &str, silent: bool) -> Receiver<Reply> {
        let (reply, receiver) = channel();
        let _ = self.tx.send(Message::Request {
            line: line.to_string(),
            silent,
            reply,
        });
        receiver
    }
}

pub fn spawn(firewall: impl Firewall, host: impl Host) -> Handle {
    let (tx, rx) = channel();
    let (wake_tx, wake_rx) = channel();
    let settings_path = settings::default_path();
    let (settings, notes) = settings_path
        .as_deref()
        .map_or_else(Default::default, settings::load);
    notes.into_iter().for_each(log::line);
    let fetch_tx = tx.clone();
    let minutes = settings.refresh_minutes;
    thread::spawn(move || fetch_loop(&fetch_tx, &wake_rx, minutes));
    let monitor_tx = tx.clone();
    let control = monitor::spawn(
        settings.ping_every,
        settings.log_path.clone(),
        move |state| monitor_tx.send(Message::Monitor(state)).is_ok(),
    );
    let mut session = Session::new(firewall, host, wake_tx);
    session.settings = settings;
    session.settings_path = settings_path;
    session.control = Some(control);
    let handle = Handle {
        tx,
        servers: session.servers.clone(),
    };
    thread::spawn(move || session.run(&rx));
    handle
}

/// A session with no fetching and no monitor, so UI tests need no network and send no pings.
#[cfg(test)]
pub fn spawn_offline(firewall: impl Firewall, host: impl Host) -> Handle {
    let (tx, rx) = channel();
    let (wake, wake_rx) = channel();
    let session = Session::new(firewall, host, wake);
    let handle = Handle {
        tx,
        servers: session.servers.clone(),
    };
    thread::spawn(move || {
        let _keep_open = wake_rx;
        session.run(&rx);
    });
    handle
}

/// Spreads refreshes out so many clients don't hit the API in lockstep.
fn jitter() -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |now| now.subsec_nanos());
    Duration::from_millis(u64::from(nanos % MAX_JITTER_MS))
}

fn refresh_every(minutes: u32) -> Duration {
    if minutes == 0 {
        MANUAL
    } else {
        Duration::from_secs(u64::from(minutes) * 60)
    }
}

/// Keeps each app's last good config, so one failed fetch never shrinks the merged relay set.
fn fetch_loop(tx: &Sender<Message>, wake: &Receiver<Wake>, minutes: u32) {
    let client = Client::new();
    let mut refresh = refresh_every(minutes);
    let mut cache: [Option<Pops>; sdr::APPS.len()] = Default::default();
    let mut backoff = RETRY_MIN;
    loop {
        let fetched_at = Instant::now();
        let mut all_ok = true;
        for (slot, app) in cache.iter_mut().zip(sdr::APPS) {
            let result = client
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|client| client.config(app));
            match result {
                Ok(pops) => {
                    log!("fetch app {app}: {} sites with relays", pops.len());
                    *slot = Some(pops);
                }
                Err(error) => {
                    log!("fetch app {app}: {error}");
                    all_ok = false;
                }
            }
        }
        let message = if cache.iter().any(Option::is_some) {
            Message::Relays(sdr::merge(cache.iter().flatten()))
        } else {
            Message::Offline
        };
        if tx.send(message).is_err() {
            return;
        }
        let delay = if all_ok {
            backoff = RETRY_MIN;
            refresh
        } else {
            let current = backoff;
            backoff = (backoff * 2).min(RETRY_MAX);
            current
        };
        let mut next = fetched_at + delay + jitter();
        let earliest = fetched_at + MIN_FETCH_GAP;
        loop {
            match wake.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(Wake::Soon) => next = next.min(earliest),
                Ok(Wake::Interval(minutes)) => {
                    refresh = refresh_every(minutes);
                    if all_ok {
                        next = fetched_at + refresh + jitter();
                    }
                }
                Ok(Wake::Now) | Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

struct Report {
    every: Duration,
    next: Instant,
}

struct Session<F, H> {
    firewall: F,
    host: H,
    pops: Pops,
    pick: String,
    backup: Option<String>,
    offline: bool,
    error: Option<String>,
    report: Option<Report>,
    wake: Sender<Wake>,
    servers: Arc<Mutex<Vec<ServerName>>>,
    settings: Settings,
    /// `None` keeps settings for this session only.
    settings_path: Option<PathBuf>,
    control: Option<Sender<Control>>,
    monitor: Monitor,
    sent_targets: Option<(String, Vec<Ipv4Addr>)>,
    game_running: fn() -> bool,
}

impl<F: Firewall, H: Host> Session<F, H> {
    /// Recovers the pick from the firewall, so reopening mid-session picks up where it left off.
    fn new(firewall: F, host: H, wake: Sender<Wake>) -> Self {
        let (pick, backup) = sdr::sites_in_stamp(&firewall.stamp().unwrap_or_default());
        Self {
            firewall,
            host,
            pops: Pops::new(),
            pick,
            backup,
            offline: false,
            error: None,
            report: None,
            wake,
            servers: Arc::default(),
            settings: Settings::default(),
            settings_path: None,
            control: None,
            monitor: Monitor::default(),
            sent_targets: None,
            game_running: diag::game_running,
        }
    }

    fn run(mut self, rx: &Receiver<Message>) {
        log!("start: active {:?}", self.open_sites().join(" + "));
        self.sync_monitor();
        self.host.changed();
        loop {
            let wait = self.report.as_ref().map_or(Duration::MAX, |report| {
                report.next.saturating_duration_since(Instant::now())
            });
            match rx.recv_timeout(wait) {
                Ok(Message::Request {
                    line,
                    silent,
                    reply,
                }) => self.request(&line, silent, &reply),
                Ok(Message::Relays(pops)) => {
                    self.relays(pops);
                    self.host.changed();
                }
                Ok(Message::Offline) => {
                    self.offline = true;
                    self.host.changed();
                }
                Ok(Message::Monitor(state)) => {
                    if state != self.monitor {
                        self.monitor = state;
                        self.host.changed();
                    }
                }
                Err(RecvTimeoutError::Timeout) => self.report_tick(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn request(&mut self, line: &str, silent: bool, reply: &Sender<Reply>) {
        if !silent {
            log!("> {line}");
        }
        let result = command::parse(line).and_then(|command| self.execute(command));
        if !silent {
            match &result {
                Ok(output) => output.render().into_iter().for_each(log::line),
                Err(error) => log!("error: {error}"),
            }
            // Even a failure counts: a dropdown may need to snap back to what's really active.
            self.host.changed();
        }
        let _ = reply.send(result);
    }

    fn relays(&mut self, pops: Pops) {
        if pops != self.pops {
            let relays: usize = pops.values().map(|pop| pop.ips.len()).sum();
            log!("relays: {} sites, {relays} addresses", pops.len());
        }
        *self.servers.lock().unwrap() = pops
            .iter()
            .map(|(code, pop)| (code.clone(), pop.desc.clone()))
            .collect();
        self.pops = pops;
        self.offline = false;
        if self.holding() {
            log!("relay list changed, holding the filter update while a game runs");
        } else if let Err(error) = self.reconcile() {
            // Runs even when nothing changed, which restores filters removed behind our back; an
            // unchanged stamp means no write.
            log!("apply failed: {error}");
        }
        self.sync_monitor();
    }

    /// Whether a background relay update should wait for the game to close. Only the same pick
    /// with new relay data waits. Missing filters are always repaired.
    fn holding(&self) -> bool {
        if !self.settings.hold_in_game {
            return false;
        }
        let open = self.open_sites();
        if open.is_empty() || !open.iter().all(|site| self.pops.contains_key(*site)) {
            return false;
        }
        let Some(current) = self.firewall.stamp() else {
            return false;
        };
        let wanted = sdr::stamp(&open, &sdr::blocklist(&self.pops, &open));
        current != wanted
            && sdr::sites_in_stamp(&current) == (self.pick.clone(), self.backup.clone())
            && (self.game_running)()
    }

    /// Tells the monitor which relays to ping, when that has changed.
    fn sync_monitor(&mut self) {
        let Some(control) = &self.control else {
            return;
        };
        let ips = self
            .pops
            .get(&self.pick)
            .map_or_else(Vec::new, |pop| pop.ips.iter().copied().collect());
        let targets = (self.pick.clone(), ips);
        if self.sent_targets.as_ref() != Some(&targets) {
            let _ = control.send(Control::Targets {
                pick: targets.0.clone(),
                ips: targets.1.clone(),
            });
            self.sent_targets = Some(targets);
        }
    }

    fn execute(&mut self, command: Command) -> Reply {
        match command {
            Command::Help { topic } => command::help(topic.as_deref()).map(Output::Lines),
            Command::ListServers { filter } => Ok(self.list_servers(filter.as_deref())),
            Command::ListRelays { server } => self.list_relays(server.as_deref()),
            Command::ShowStatus => Ok(Output::Status(self.status())),
            Command::SetPick { server, backup } => self.set_pick(server, backup),
            Command::ClearPick => self.clear_pick(),
            Command::RefreshRelays => {
                let _ = self.wake.send(Wake::Now);
                Ok(Output::line("refreshing relays".to_string()))
            }
            Command::ShowRoutes { server } => Ok(Output::Lines(diag::routes(
                &self.targets(server.as_deref())?,
            ))),
            Command::PingRelays { server } => Ok(Output::Lines(diag::pings(
                &self.targets(server.as_deref())?,
            ))),
            Command::ShowProcesses => Ok(Output::line(diag::processes())),
            Command::ShowReport => Ok(Output::Lines(diag::report(&self.pops, &self.open_sites()))),
            Command::StartReport { every } => self.start_report(every),
            Command::StopReport => {
                self.stop_report();
                Ok(Output::line("report stopped".to_string()))
            }
            Command::ToggleConsole => {
                if !self.host.toggle_console() {
                    self.stop_report();
                    Ok(Output::Lines(Vec::new()))
                } else if self.settings.report_on_open {
                    self.start_report(None)
                } else {
                    Ok(Output::Lines(Vec::new()))
                }
            }
            Command::ClearScreen => {
                log::clear();
                Ok(Output::Lines(Vec::new()))
            }
            Command::ShowConfig => Ok(Output::Config(self.settings.rows())),
            Command::SetConfig { key, value } => self.set_config(&key, &value),
            Command::ClearConfig { key } => self.clear_config(key.as_deref()),
            Command::AnalyzeLog => self.analyze_log(),
            Command::ShowGuide => Ok(Output::Lines(guide::lines())),
            Command::Quit => {
                if self.settings.clear_on_exit
                    && !self.pick.is_empty()
                    && let Err(error) = self.clear_pick()
                {
                    log!("clear on exit failed: {error}");
                }
                self.host.quit();
                Ok(Output::Lines(Vec::new()))
            }
        }
    }

    fn set_config(&mut self, key: &str, value: &str) -> Reply {
        if key == "backup" && !value.trim().eq_ignore_ascii_case("none") && !self.pops.is_empty() {
            self.known(&value.trim().to_lowercase())?;
        }
        self.settings.set(key, value)?;
        self.apply_setting(key);
        self.save_settings()?;
        let shown = self.settings.rows().into_iter().find(|row| row.key == key);
        Ok(Output::line(format!(
            "{key} = {}",
            shown.map_or(String::new(), |row| row.value)
        )))
    }

    fn clear_config(&mut self, key: Option<&str>) -> Reply {
        let keys: Vec<&str> = match key {
            Some(key) => vec![key],
            None => settings::names().map(|(name, _)| name).collect(),
        };
        for key in &keys {
            self.settings.reset(key)?;
            self.apply_setting(key);
        }
        self.save_settings()?;
        Ok(Output::line(match key {
            Some(key) => format!("{key} is back to its default"),
            None => "all settings are back to their defaults".to_string(),
        }))
    }

    /// A changed setting takes effect now, except that a default backup waits for the next pick so
    /// that changing it doesn't rewrite filters.
    fn apply_setting(&mut self, key: &str) {
        match key {
            "ping_every" => self.send_control(Control::PingEvery(self.settings.ping_every)),
            "log_path" => self.send_control(Control::LogPath(self.settings.log_path.clone())),
            "refresh_minutes" => {
                let _ = self
                    .wake
                    .send(Wake::Interval(self.settings.refresh_minutes));
            }
            "report_every" => {
                if let Some(report) = &mut self.report {
                    report.every = Duration::from_secs(u64::from(self.settings.report_every));
                }
            }
            _ => {}
        }
    }

    fn send_control(&self, message: Control) {
        if let Some(control) = &self.control {
            let _ = control.send(message);
        }
    }

    fn save_settings(&self) -> Result<(), String> {
        let Some(path) = &self.settings_path else {
            return Ok(());
        };
        settings::save(path, &self.settings)
            .map_err(|error| format!("applied for now, but it couldn't be saved: {error}"))
    }

    fn analyze_log(&self) -> Reply {
        let (text, mut context) = diag::read_log(self.settings.log_path.as_deref())?;
        context.pick.clone_from(&self.pick);
        context.backup.clone_from(&self.backup);
        context.exitlag = self.monitor.exitlag;
        if let Direct::Ms(ms) = self.monitor.direct {
            context.direct_ms = Some(ms);
        }
        Ok(Output::Lines(analysis::report(
            &analysis::parse(&text),
            &context,
        )))
    }

    fn list_servers(&self, filter: Option<&str>) -> Output {
        let filter = filter.unwrap_or_default().to_lowercase();
        Output::Servers(
            self.pops
                .iter()
                .filter(|(code, pop)| {
                    code.contains(&filter) || pop.desc.to_lowercase().contains(&filter)
                })
                .map(|(code, pop)| Server {
                    code: code.clone(),
                    desc: pop.desc.clone(),
                    relays: pop.ips.len(),
                })
                .collect(),
        )
    }

    fn list_relays(&self, server: Option<&str>) -> Reply {
        if let Some(server) = server {
            self.known(server)?;
        }
        let lines = self
            .pops
            .iter()
            .filter(|(code, _)| server.is_none_or(|server| server == code.as_str()))
            .map(|(code, pop)| {
                let ips: Vec<String> = pop.ips.iter().map(ToString::to_string).collect();
                format!("{code:<6} {}", ips.join(" "))
            })
            .collect();
        Ok(Output::Lines(lines))
    }

    fn status(&self) -> Status {
        let blocks = sdr::blocklist(&self.pops, &self.open_sites());
        Status {
            pick: self.pick.clone(),
            pick_desc: self.pops.get(&self.pick).map(|pop| pop.desc.clone()),
            backup: self.backup.clone(),
            blocked: blocks.values().map(Vec::len).sum(),
            sites_blocked: blocks.len(),
            loaded: !self.pops.is_empty(),
            offline: self.offline,
            error: self.error.clone(),
            monitor: self.monitor.clone(),
        }
    }

    /// A pick states the whole intent: its backup, or `none` for no backup, or the default backup
    /// if it names neither. A default that can't be used falls back to strict with a note, because
    /// a stale setting must never stop a pick.
    fn set_pick(&mut self, server: String, backup: Option<String>) -> Reply {
        self.known(&server)?;
        let backup = match backup.as_deref() {
            Some("none") => None,
            Some(named) => {
                self.known(named)?;
                backup
            }
            None => self.default_backup(&server),
        };
        let changed = server != self.pick || backup != self.backup;
        if server != self.pick {
            // A figure for the last pick would be wrong until the monitor measures this one.
            self.monitor.direct = Direct::Waiting;
        }
        self.pick = server;
        self.backup = backup;
        let result = self.reconcile();
        self.sync_monitor();
        result?;
        if changed {
            let _ = self.wake.send(Wake::Soon);
        }
        Ok(Output::Status(self.status()))
    }

    fn default_backup(&self, server: &str) -> Option<String> {
        let backup = self.settings.backup.as_ref()?;
        if backup == server {
            log!("default backup {backup} is the pick, so this pick is strict");
            None
        } else if self.pops.contains_key(backup) {
            Some(backup.clone())
        } else {
            log!("default backup {backup} isn't in the relay list, so this pick is strict");
            None
        }
    }

    fn clear_pick(&mut self) -> Reply {
        self.monitor.direct = Direct::NoPick;
        self.pick.clear();
        self.backup = None;
        let result = self.reconcile();
        self.sync_monitor();
        result?;
        Ok(Output::Status(self.status()))
    }

    /// The sites to keep open: none, the pick, or the pick and its backup.
    fn open_sites(&self) -> Vec<&str> {
        if self.pick.is_empty() {
            return Vec::new();
        }
        std::iter::once(self.pick.as_str())
            .chain(self.backup.as_deref())
            .collect()
    }

    /// The relays a diagnostic command should look at: the named server's, or the pick's.
    fn targets(&self, server: Option<&str>) -> Result<Vec<Ipv4Addr>, String> {
        let code = match server {
            Some(server) => server,
            None if self.pick.is_empty() => {
                return Err("nothing is picked, so name a server".to_string());
            }
            None => self.pick.as_str(),
        };
        self.known(code)?;
        Ok(self.pops[code].ips.iter().copied().collect())
    }

    fn known(&self, server: &str) -> Result<(), String> {
        if self.pops.contains_key(server) {
            Ok(())
        } else if self.pops.is_empty() {
            Err("relays aren't loaded yet, try again in a moment".to_string())
        } else {
            Err(format!("unknown server '{server}'. Try 'list servers'"))
        }
    }

    /// Makes the firewall match the pick. Filter writes make Windows re-check every live flow, so
    /// this only writes when the stamp differs. Data problems leave the current block in place.
    fn reconcile(&mut self) -> Result<(), String> {
        let open = self.open_sites();
        let result = if open.is_empty() {
            self.firewall.clear()
        } else if !open.iter().all(|site| self.pops.contains_key(*site)) {
            if !self.pops.is_empty() {
                log!(
                    "{:?} not all in config; keeping current block",
                    open.join(" + ")
                );
            }
            Ok(())
        } else {
            let blocks = sdr::blocklist(&self.pops, &open);
            let stamp = sdr::stamp(&open, &blocks);
            if self.firewall.stamp().as_deref() == Some(&stamp) {
                Ok(())
            } else {
                log!("apply: {stamp}, {} site filters", blocks.len());
                self.firewall.apply(&stamp, &blocks)
            }
        };
        self.error = result.as_ref().err().cloned();
        result
    }

    fn start_report(&mut self, every: Option<u64>) -> Reply {
        let secs = every.unwrap_or(u64::from(self.settings.report_every));
        if let Err(error) = self.firewall.watch_drops(true) {
            log!("drop watcher: {error}");
        }
        self.report = Some(Report {
            every: Duration::from_secs(secs),
            next: Instant::now(),
        });
        Ok(Output::line(format!("report every {secs} s")))
    }

    fn stop_report(&mut self) {
        self.report = None;
        if let Err(error) = self.firewall.watch_drops(false) {
            log!("drop watcher: {error}");
        }
    }

    fn report_tick(&mut self) {
        diag::report(&self.pops, &self.open_sites())
            .into_iter()
            .for_each(log::line);
        if let Some(report) = &mut self.report {
            report.next = Instant::now() + report.every;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakeState {
        stamp: Mutex<Option<String>>,
        writes: AtomicUsize,
        fail_apply: AtomicBool,
        watching: AtomicBool,
    }

    #[derive(Clone, Default)]
    struct FakeFirewall(Arc<FakeState>);

    impl Firewall for FakeFirewall {
        fn stamp(&self) -> Option<String> {
            self.0.stamp.lock().unwrap().clone()
        }
        fn apply(&self, stamp: &str, _blocks: &Blocks) -> Result<(), String> {
            if self.0.fail_apply.load(Ordering::Relaxed) {
                return Err("denied".to_string());
            }
            self.0.writes.fetch_add(1, Ordering::Relaxed);
            *self.0.stamp.lock().unwrap() = Some(stamp.to_string());
            Ok(())
        }
        fn clear(&self) -> Result<(), String> {
            *self.0.stamp.lock().unwrap() = None;
            Ok(())
        }
        fn watch_drops(&self, on: bool) -> Result<(), String> {
            self.0.watching.store(on, Ordering::Relaxed);
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct FakeHost {
        visible: Arc<AtomicBool>,
        quit: Arc<AtomicBool>,
    }

    impl Host for FakeHost {
        fn toggle_console(&self) -> bool {
            !self.visible.fetch_not(Ordering::Relaxed)
        }
        fn quit(&self) {
            self.quit.store(true, Ordering::Relaxed);
        }
        fn changed(&self) {}
    }

    type TestSession = Session<FakeFirewall, FakeHost>;

    fn pops() -> Pops {
        sdr::parse(
            r#"{"pops":{
                "fra":{"desc":"Frankfurt (Germany)","relays":[{"ipv4":"1.1.1.1"},{"ipv4":"1.1.1.2"}]},
                "waw":{"desc":"Warsaw (Poland)","relays":[{"ipv4":"2.2.2.2"}]},
                "jnb":{"desc":"Johannesburg (South Africa)","relays":[{"ipv4":"3.3.3.3"}]}
            }}"#,
        )
        .unwrap()
    }

    fn session() -> (TestSession, FakeFirewall, FakeHost, Receiver<Wake>) {
        let firewall = FakeFirewall::default();
        let host = FakeHost::default();
        let (wake, wake_rx) = channel();
        let mut session = Session::new(firewall.clone(), host.clone(), wake);
        session.relays(pops());
        (session, firewall, host, wake_rx)
    }

    fn run(session: &mut TestSession, line: &str) -> Reply {
        command::parse(line).and_then(|command| session.execute(command))
    }

    fn writes(firewall: &FakeFirewall) -> usize {
        firewall.0.writes.load(Ordering::Relaxed)
    }

    #[test]
    fn a_pick_applies_once_and_repeats_write_nothing() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        assert_eq!(writes(&firewall), 1);
        assert!(firewall.stamp().unwrap().starts_with("fra "));

        run(&mut session, "set pick fra").unwrap();
        session.relays(pops());
        assert_eq!(writes(&firewall), 1, "same result must not rewrite filters");
    }

    #[test]
    fn changed_relays_rewrite_the_block() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        let mut changed = pops();
        changed.remove("waw");
        session.relays(changed);
        assert_eq!(writes(&firewall), 2);
    }

    #[test]
    fn a_backup_stays_open_and_is_recovered_on_reopen() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra --backup waw").unwrap();
        assert!(firewall.stamp().unwrap().starts_with("fra+waw "));
        let status = session.status();
        assert_eq!(status.backup.as_deref(), Some("waw"));
        assert_eq!((status.blocked, status.sites_blocked), (1, 1));
        assert!(Output::Status(status).render()[0].contains("backup waw, 1 relays blocked"));

        let (wake, _wake_rx) = channel();
        let reopened = Session::new(firewall, FakeHost::default(), wake);
        assert_eq!(reopened.pick, "fra");
        assert_eq!(reopened.backup.as_deref(), Some("waw"));
    }

    #[test]
    fn a_plain_pick_returns_to_strict_blocking() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra --backup waw").unwrap();
        run(&mut session, "set pick fra").unwrap();
        assert_eq!(session.backup, None);
        assert!(firewall.stamp().unwrap().starts_with("fra "));
        assert_eq!(writes(&firewall), 2);
    }

    #[test]
    fn repeating_a_backup_writes_nothing_and_changing_it_does() {
        let (mut session, firewall, _, wake) = session();
        run(&mut session, "set pick fra --backup waw").unwrap();
        assert!(matches!(wake.try_recv(), Ok(Wake::Soon)));
        run(&mut session, "set pick fra --backup waw").unwrap();
        assert_eq!(writes(&firewall), 1);
        assert!(wake.try_recv().is_err());
        run(&mut session, "set pick fra --backup jnb").unwrap();
        assert_eq!(writes(&firewall), 2);
        assert!(matches!(wake.try_recv(), Ok(Wake::Soon)));
    }

    #[test]
    fn an_unknown_backup_errors_and_changes_nothing() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        let before = firewall.stamp();
        let error = run(&mut session, "set pick waw --backup nope").unwrap_err();
        assert!(error.contains("unknown server 'nope'"), "{error}");
        assert_eq!(firewall.stamp(), before);
        assert_eq!(session.pick, "fra");
    }

    #[test]
    fn a_backup_leaving_the_list_keeps_the_block() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra --backup waw").unwrap();
        let before = firewall.stamp();
        let mut without_backup = pops();
        without_backup.remove("waw");
        session.relays(without_backup);
        assert_eq!(firewall.stamp(), before);
        assert_eq!(writes(&firewall), 1);
    }

    #[test]
    fn clear_pick_removes_the_backup_too() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra --backup waw").unwrap();
        run(&mut session, "clear pick").unwrap();
        assert_eq!(firewall.stamp(), None);
        assert_eq!(session.backup, None);
        assert!(session.open_sites().is_empty());
    }

    #[test]
    fn unknown_server_errors_and_changes_nothing() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        let before = firewall.stamp();
        let error = run(&mut session, "set pick nope").unwrap_err();
        assert!(error.contains("unknown server 'nope'"), "{error}");
        assert_eq!(firewall.stamp(), before);
        assert_eq!(session.pick, "fra");
    }

    #[test]
    fn picking_before_relays_load_says_so() {
        let (wake, _wake_rx) = channel();
        let mut session = Session::new(FakeFirewall::default(), FakeHost::default(), wake);
        let error = run(&mut session, "set pick fra").unwrap_err();
        assert!(error.contains("aren't loaded"), "{error}");
    }

    #[test]
    fn clear_pick_removes_the_block() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        run(&mut session, "clear pick").unwrap();
        assert_eq!(firewall.stamp(), None);
        assert_eq!(session.pick, "");
    }

    #[test]
    fn a_failed_fetch_keeps_the_block() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        let before = firewall.stamp();
        // The fetch loop reports failure as Offline, or a list without the pick.
        session.offline = true;
        let mut without_pick = pops();
        without_pick.remove("fra");
        session.relays(without_pick);
        assert_eq!(firewall.stamp(), before);
        assert_eq!(writes(&firewall), 1);
        assert_eq!(session.pick, "fra");
    }

    #[test]
    fn firewall_errors_reach_the_caller_and_the_status() {
        let (mut session, firewall, _, _wake) = session();
        firewall.0.fail_apply.store(true, Ordering::Relaxed);
        assert_eq!(run(&mut session, "set pick fra").unwrap_err(), "denied");
        assert_eq!(session.status().error.as_deref(), Some("denied"));
        firewall.0.fail_apply.store(false, Ordering::Relaxed);
        run(&mut session, "set pick fra").unwrap();
        assert_eq!(session.status().error, None);
    }

    #[test]
    fn reopening_recovers_the_pick_from_the_firewall() {
        let firewall = FakeFirewall::default();
        *firewall.0.stamp.lock().unwrap() = Some("waw 0123".to_string());
        let (wake, _wake_rx) = channel();
        let session = Session::new(firewall, FakeHost::default(), wake);
        assert_eq!(session.pick, "waw");
    }

    #[test]
    fn status_reports_what_is_blocked() {
        let (mut session, _, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        let status = session.status();
        assert_eq!(status.pick_desc.as_deref(), Some("Frankfurt (Germany)"));
        assert_eq!((status.blocked, status.sites_blocked), (2, 2));
        assert!(Output::Status(status).render()[0].contains("2 relays blocked"));
    }

    #[test]
    fn list_servers_filters_by_code_or_name() {
        let (session, _, _, _wake) = session();
        let codes = |filter| match session.list_servers(filter) {
            Output::Servers(servers) => servers
                .into_iter()
                .map(|server| server.code)
                .collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        assert_eq!(codes(None).len(), 3);
        assert_eq!(codes(Some("SOUTH")), ["jnb"]);
        assert_eq!(codes(Some("wa")), ["waw"]);
        assert!(codes(Some("nowhere")).is_empty());
    }

    #[test]
    fn list_relays_can_narrow_to_one_server() {
        let (mut session, _, _, _wake) = session();
        let Output::Lines(all) = run(&mut session, "list relays").unwrap() else {
            panic!();
        };
        assert_eq!(all.len(), 3);
        let Output::Lines(one) = run(&mut session, "list relays fra").unwrap() else {
            panic!();
        };
        assert_eq!(one, ["fra    1.1.1.1 1.1.1.2"]);
        assert!(run(&mut session, "list relays nope").is_err());
    }

    #[test]
    fn diagnostics_need_a_target() {
        let (mut session, _, _, _wake) = session();
        assert!(
            run(&mut session, "show routes")
                .unwrap_err()
                .contains("name a server")
        );
        assert!(
            run(&mut session, "ping relays nope")
                .unwrap_err()
                .contains("unknown")
        );
    }

    #[test]
    fn the_console_toggle_drives_the_report_and_drop_watching() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "toggle console").unwrap();
        assert!(session.report.is_some());
        assert!(firewall.0.watching.load(Ordering::Relaxed));
        run(&mut session, "toggle console").unwrap();
        assert!(session.report.is_none());
        assert!(!firewall.0.watching.load(Ordering::Relaxed));
    }

    #[test]
    fn report_interval_is_configurable() {
        let (mut session, _, _, _wake) = session();
        let output = run(&mut session, "start report --every 30").unwrap();
        assert_eq!(output, Output::line("report every 30 s".to_string()));
        assert_eq!(
            session.report.as_ref().unwrap().every,
            Duration::from_secs(30)
        );
        run(&mut session, "stop report").unwrap();
        assert!(session.report.is_none());
    }

    #[test]
    fn refresh_and_pick_wake_the_fetch_loop_appropriately() {
        let (mut session, _, _, wake) = session();
        run(&mut session, "refresh relays").unwrap();
        assert!(matches!(wake.try_recv(), Ok(Wake::Now)));
        run(&mut session, "set pick fra").unwrap();
        assert!(matches!(wake.try_recv(), Ok(Wake::Soon)));
        run(&mut session, "set pick fra").unwrap();
        assert!(
            wake.try_recv().is_err(),
            "an unchanged pick needs no refresh"
        );
    }

    #[test]
    fn a_default_backup_applies_to_plain_picks() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set config backup waw").unwrap();
        run(&mut session, "set pick fra").unwrap();
        assert_eq!(session.backup.as_deref(), Some("waw"));
        assert!(firewall.stamp().unwrap().starts_with("fra+waw "));
    }

    #[test]
    fn backup_none_overrides_the_default_for_one_pick() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set config backup waw").unwrap();
        run(&mut session, "set pick fra --backup none").unwrap();
        assert_eq!(session.backup, None);
        assert!(firewall.stamp().unwrap().starts_with("fra "));
    }

    #[test]
    fn a_stale_default_backup_never_stops_a_pick() {
        let (mut session, firewall, _, _wake) = session();
        session.settings.backup = Some("gone".to_string());
        run(&mut session, "set pick fra").unwrap();
        assert_eq!(session.backup, None);
        assert!(firewall.stamp().unwrap().starts_with("fra "));

        session.settings.backup = Some("fra".to_string());
        run(&mut session, "set pick fra").unwrap();
        assert_eq!(session.backup, None, "the pick can't be its own backup");
    }

    #[test]
    fn changing_the_default_backup_writes_no_filters() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        run(&mut session, "set config backup waw").unwrap();
        assert_eq!(writes(&firewall), 1);
        assert_eq!(session.backup, None, "it waits for the next pick");
    }

    #[test]
    fn set_config_validates_and_reports() {
        let (mut session, _, _, _wake) = session();
        let output = run(&mut session, "set config ping_every 30").unwrap();
        assert_eq!(output, Output::line("ping_every = 30".to_string()));
        assert_eq!(session.settings.ping_every, 30);
        assert!(
            run(&mut session, "set config ping_every 999")
                .unwrap_err()
                .contains("0 to 300")
        );
        assert_eq!(session.settings.ping_every, 30);
        assert!(
            run(&mut session, "set config backup nope")
                .unwrap_err()
                .contains("unknown server 'nope'")
        );
        assert_eq!(session.settings.backup, None);
    }

    #[test]
    fn show_config_returns_typed_rows() {
        let (mut session, _, _, _wake) = session();
        let Output::Config(rows) = run(&mut session, "show config").unwrap() else {
            panic!();
        };
        assert!(rows.iter().any(|row| row.key == "hold_in_game"));
        let lines = Output::Config(rows).render();
        assert!(lines.iter().any(|line| line.starts_with("ping_every")));
    }

    #[test]
    fn clear_config_restores_one_or_all() {
        let (mut session, _, _, _wake) = session();
        run(&mut session, "set config ping_every 30").unwrap();
        run(&mut session, "set config report_every 60").unwrap();
        run(&mut session, "clear config ping_every").unwrap();
        assert_eq!(session.settings.ping_every, 5);
        assert_eq!(session.settings.report_every, 60);
        run(&mut session, "clear config").unwrap();
        assert_eq!(session.settings, Settings::default());
    }

    #[test]
    fn settings_changes_reach_what_they_control() {
        let (mut session, _, _, wake) = session();
        let (control, controls) = channel();
        session.control = Some(control);

        run(&mut session, "set config ping_every 30").unwrap();
        assert!(matches!(controls.try_recv(), Ok(Control::PingEvery(30))));

        run(&mut session, "set config log_path D:\\a b\\console.log").unwrap();
        assert!(matches!(
            controls.try_recv(),
            Ok(Control::LogPath(Some(path))) if path == "D:\\a b\\console.log"
        ));

        run(&mut session, "set config refresh_minutes 0").unwrap();
        assert!(matches!(wake.try_recv(), Ok(Wake::Interval(0))));

        run(&mut session, "set config report_every 42").unwrap();
        run(&mut session, "start report").unwrap();
        assert_eq!(
            session.report.as_ref().unwrap().every,
            Duration::from_secs(42)
        );
        run(&mut session, "set config report_every 7").unwrap();
        assert_eq!(
            session.report.as_ref().unwrap().every,
            Duration::from_secs(7)
        );
    }

    #[test]
    fn the_monitor_follows_the_pick_and_only_hears_changes() {
        let (mut session, _, _, _wake) = session();
        let (control, controls) = channel();
        session.control = Some(control);
        run(&mut session, "set pick fra").unwrap();
        let Ok(Control::Targets { pick, ips }) = controls.try_recv() else {
            panic!("the monitor should hear the pick");
        };
        assert_eq!(pick, "fra");
        assert_eq!(ips.len(), 2);
        run(&mut session, "set pick fra").unwrap();
        session.relays(pops());
        assert!(controls.try_recv().is_err(), "nothing changed");
        run(&mut session, "clear pick").unwrap();
        let Ok(Control::Targets { pick, ips }) = controls.try_recv() else {
            panic!("the monitor should hear the clear");
        };
        assert!(pick.is_empty() && ips.is_empty());
    }

    #[test]
    fn the_console_can_open_without_starting_the_report() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set config report_on_open off").unwrap();
        run(&mut session, "toggle console").unwrap();
        assert!(session.report.is_none());
        assert!(!firewall.0.watching.load(Ordering::Relaxed));
    }

    fn changed_relays() -> Pops {
        let mut changed = pops();
        changed.remove("waw");
        changed
    }

    #[test]
    fn hold_in_game_delays_a_background_rewrite_until_the_game_closes() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set config hold_in_game on").unwrap();
        run(&mut session, "set pick fra").unwrap();
        session.game_running = || true;
        session.relays(changed_relays());
        assert_eq!(writes(&firewall), 1, "held while the game runs");
        session.game_running = || false;
        session.relays(changed_relays());
        assert_eq!(writes(&firewall), 2, "applied once it's closed");
    }

    #[test]
    fn hold_in_game_never_delays_a_pick() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set config hold_in_game on").unwrap();
        session.game_running = || true;
        run(&mut session, "set pick fra").unwrap();
        run(&mut session, "set pick waw").unwrap();
        assert_eq!(writes(&firewall), 2);
        run(&mut session, "clear pick").unwrap();
        assert_eq!(firewall.stamp(), None);
    }

    #[test]
    fn hold_in_game_still_repairs_missing_filters() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set config hold_in_game on").unwrap();
        run(&mut session, "set pick fra").unwrap();
        session.game_running = || true;
        firewall.clear().unwrap();
        session.relays(pops());
        assert!(firewall.stamp().is_some(), "the block must come back");
    }

    #[test]
    fn hold_in_game_off_ignores_the_game() {
        let (mut session, firewall, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        session.game_running = || true;
        session.relays(changed_relays());
        assert_eq!(writes(&firewall), 2);
    }

    #[test]
    fn quit_can_clear_the_block_first() {
        let (mut session, firewall, host, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        run(&mut session, "set config clear_on_exit on").unwrap();
        run(&mut session, "quit").unwrap();
        assert_eq!(firewall.stamp(), None);
        assert!(host.quit.load(Ordering::Relaxed));
    }

    #[test]
    fn quit_leaves_the_block_by_default() {
        let (mut session, firewall, host, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        run(&mut session, "quit").unwrap();
        assert!(firewall.stamp().is_some());
        assert!(host.quit.load(Ordering::Relaxed));
    }

    #[test]
    fn analyze_log_reports_on_the_configured_file() {
        let (mut session, _, _, _wake) = session();
        let path = std::env::temp_dir().join(format!("dl-rs-log-{}.log", std::process::id()));
        std::fs::write(
            &path,
            "11/05 18:46:43 [SteamNetSockets] [#1 SDR server vport 0] Selecting fra#1 (1.1.1.1:1) as primary.  (Ping = 40 = 39+1+0 (front+interior+remote).  Rank=1 Score=1)\n",
        )
        .unwrap();
        session.settings.log_path = Some(path.display().to_string());
        session.pick = "fra".to_string();
        let Output::Lines(lines) = run(&mut session, "analyze log").unwrap() else {
            panic!();
        };
        std::fs::remove_file(&path).unwrap();
        assert!(
            lines
                .join("\n")
                .contains("Primary relay fra#1 is on your pick")
        );
    }

    #[test]
    fn analyze_log_explains_a_missing_file() {
        let (mut session, _, _, _wake) = session();
        session.settings.log_path = Some(r"Z:\no\such\console.log".to_string());
        let error = run(&mut session, "analyze log").unwrap_err();
        assert!(error.contains(r"Z:\no\such\console.log"), "{error}");
    }

    #[test]
    fn the_guide_is_a_command() {
        let (mut session, _, _, _wake) = session();
        let Output::Lines(lines) = run(&mut session, "show guide").unwrap() else {
            panic!();
        };
        assert!(lines.join("\n").contains("CUSTOM GAME"));
    }

    #[test]
    fn changing_the_pick_drops_the_old_direct_figure() {
        let (mut session, _, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        session.monitor.direct = Direct::Ms(40);
        run(&mut session, "set pick fra").unwrap();
        assert_eq!(
            session.monitor.direct,
            Direct::Ms(40),
            "same pick, still true"
        );
        run(&mut session, "set pick waw").unwrap();
        assert_eq!(session.monitor.direct, Direct::Waiting);
        session.monitor.direct = Direct::Ms(40);
        run(&mut session, "clear pick").unwrap();
        assert_eq!(session.monitor.direct, Direct::NoPick);
    }

    #[test]
    fn status_carries_the_counters() {
        let (mut session, _, _, _wake) = session();
        run(&mut session, "set pick fra").unwrap();
        session.monitor.direct = Direct::Ms(40);
        let status = session.status();
        assert_eq!(status.monitor.direct, Direct::Ms(40));
        let lines = Output::Status(status).render();
        assert!(lines.iter().any(|line| line.starts_with("Direct: 40 ms")));
        assert!(lines.iter().any(|line| line.starts_with("Game:")));
    }

    #[test]
    fn quit_asks_the_host_to_close() {
        let (mut session, _, host, _wake) = session();
        run(&mut session, "quit").unwrap();
        assert!(host.quit.load(Ordering::Relaxed));
    }

    #[test]
    fn completion_servers_follow_the_relay_list() {
        let (session, _, _, _wake) = session();
        let servers = session.servers.lock().unwrap();
        assert_eq!(servers.len(), 3);
        assert!(servers.contains(&("fra".to_string(), "Frankfurt (Germany)".to_string())));
    }
}
