//! The app's core: owns the firewall, relays and pick, and runs commands one at a time.
//!
//! Front-ends only send command lines to a [`Handle`]. The session knows nothing about windows; a
//! [`Host`] gives it the few things it needs from whichever front-end started it.

use crate::command::{self, Command, ServerName};
use crate::diag;
use crate::log;
use crate::sdr::{self, Blocks, Client, Pops};
use crate::wfp::Engine;
use std::net::Ipv4Addr;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Steam serves this API with `max-age` up to 300 s, so polling faster would see nothing new.
const REFRESH: Duration = Duration::from_secs(300);
/// Picking pulls the next fetch forward, but never closer than this to the last one.
const MIN_FETCH_GAP: Duration = Duration::from_secs(30);
const RETRY_MIN: Duration = Duration::from_secs(5);
const MAX_JITTER_MS: u32 = 15_000;
const REPORT_EVERY_SECS: u64 = 5;

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
    pub blocked: usize,
    pub sites_blocked: usize,
    pub loaded: bool,
    pub offline: bool,
    pub error: Option<String>,
}

/// A command's result as data, so a UI never has to parse text.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Lines(Vec<String>),
    Servers(Vec<Server>),
    Status(Status),
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
                        "pick: {pick} {desc}, {} relays blocked across {} sites",
                        status.blocked, status.sites_blocked
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
                lines
            }
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
}

enum Wake {
    /// Refresh soon, respecting the minimum gap between fetches.
    Soon,
    Now,
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

pub fn spawn(engine: Engine, host: impl Host) -> Handle {
    let (tx, rx) = channel();
    let (wake_tx, wake_rx) = channel();
    let fetch_tx = tx.clone();
    thread::spawn(move || fetch_loop(&fetch_tx, &wake_rx));
    let session = Session::new(engine, host, wake_tx);
    let handle = Handle {
        tx,
        servers: session.servers.clone(),
    };
    thread::spawn(move || session.run(&rx));
    handle
}

/// Spreads refreshes out so many clients don't hit the API in lockstep.
fn jitter() -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |now| now.subsec_nanos());
    Duration::from_millis(u64::from(nanos % MAX_JITTER_MS))
}

/// Keeps each app's last good config, so one failed fetch never shrinks the merged relay set.
fn fetch_loop(tx: &Sender<Message>, wake: &Receiver<Wake>) {
    let client = Client::new();
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
            REFRESH
        } else {
            let current = backoff;
            backoff = (backoff * 2).min(REFRESH);
            current
        };
        let mut next = fetched_at + delay + jitter();
        let earliest = fetched_at + MIN_FETCH_GAP;
        loop {
            match wake.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(Wake::Soon) => next = next.min(earliest),
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
    offline: bool,
    error: Option<String>,
    report: Option<Report>,
    wake: Sender<Wake>,
    servers: Arc<Mutex<Vec<ServerName>>>,
}

impl<F: Firewall, H: Host> Session<F, H> {
    /// Recovers the pick from the firewall, so reopening mid-session picks up where it left off.
    fn new(firewall: F, host: H, wake: Sender<Wake>) -> Self {
        let stamp = firewall.stamp().unwrap_or_default();
        let pick = stamp.split(' ').next().unwrap_or_default().to_string();
        Self {
            firewall,
            host,
            pops: Pops::new(),
            pick,
            offline: false,
            error: None,
            report: None,
            wake,
            servers: Arc::default(),
        }
    }

    fn run(mut self, rx: &Receiver<Message>) {
        log!("start: active pick {:?}", self.pick);
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
        // Runs even when nothing changed, which restores filters removed behind our back; an
        // unchanged stamp means no write.
        if let Err(error) = self.reconcile() {
            log!("apply failed: {error}");
        }
    }

    fn execute(&mut self, command: Command) -> Reply {
        match command {
            Command::Help { topic } => command::help(topic.as_deref()).map(Output::Lines),
            Command::ListServers { filter } => Ok(self.list_servers(filter.as_deref())),
            Command::ListRelays { server } => self.list_relays(server.as_deref()),
            Command::ShowStatus => Ok(Output::Status(self.status())),
            Command::SetPick { server } => self.set_pick(server),
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
            Command::ShowReport => Ok(Output::Lines(diag::report(&self.pops, &self.pick))),
            Command::StartReport { every } => self.start_report(every),
            Command::StopReport => {
                self.stop_report();
                Ok(Output::line("report stopped".to_string()))
            }
            Command::ToggleConsole => {
                if self.host.toggle_console() {
                    self.start_report(None)
                } else {
                    self.stop_report();
                    Ok(Output::Lines(Vec::new()))
                }
            }
            Command::ClearScreen => {
                log::clear();
                Ok(Output::Lines(Vec::new()))
            }
            Command::Quit => {
                self.host.quit();
                Ok(Output::Lines(Vec::new()))
            }
        }
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
        let blocks = sdr::blocklist(&self.pops, &self.pick);
        Status {
            pick: self.pick.clone(),
            pick_desc: self.pops.get(&self.pick).map(|pop| pop.desc.clone()),
            blocked: blocks.values().map(Vec::len).sum(),
            sites_blocked: blocks.len(),
            loaded: !self.pops.is_empty(),
            offline: self.offline,
            error: self.error.clone(),
        }
    }

    fn set_pick(&mut self, server: String) -> Reply {
        self.known(&server)?;
        let changed = server != self.pick;
        self.pick = server;
        self.reconcile()?;
        if changed {
            let _ = self.wake.send(Wake::Soon);
        }
        Ok(Output::Status(self.status()))
    }

    fn clear_pick(&mut self) -> Reply {
        self.pick.clear();
        self.reconcile()?;
        Ok(Output::Status(self.status()))
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
        let result = if self.pick.is_empty() {
            self.firewall.clear()
        } else if !self.pops.contains_key(&self.pick) {
            if !self.pops.is_empty() {
                log!("pick {:?} not in config; keeping current block", self.pick);
            }
            Ok(())
        } else {
            let blocks = sdr::blocklist(&self.pops, &self.pick);
            let stamp = sdr::stamp(&self.pick, &blocks);
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
        let secs = every.unwrap_or(REPORT_EVERY_SECS);
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
        diag::report(&self.pops, &self.pick)
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
