//! The two ping counters: direct pings to the pick, and the game's own figure from its log.
//!
//! This runs on its own thread so a slow ping never holds up commands. The direct figure is a real
//! measurement of this PC's path. The game's figure is the last one it wrote, and it's the only one
//! that sees a routing tool such as ExitLag, if one is in use. Nothing here needs ExitLag or Steam.

use crate::analysis::{GameView, LiveRoute, PingEntry, Route, RouteState};
use crate::diag;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// A file's first bytes identify it, because the game rewrites the log on each launch.
const HEAD_BYTES: usize = 64;
/// The log only needs checking about as often as someone could notice the counter change.
const LOG_EVERY: Duration = Duration::from_secs(5);
/// Looking for a log that isn't there means reading the registry and Steam's files.
const SEARCH_EVERY: Duration = Duration::from_secs(30);
/// On a first read, skip most of a huge log. Its end holds the current state.
const MAX_FIRST_READ: u64 = 16 << 20;
/// The last value holds for this many misses, because single ICMP failures are common here.
const HOLD_MISSES: u32 = 2;
/// How long a running game's log can stay unchanged before it counts as not being written. A logging
/// game went 60 s without a line in testing, so this is well past that.
const SILENT_AFTER: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Direct {
    /// `ping_every` is 0.
    Off,
    #[default]
    NoPick,
    Waiting,
    Ms(u32),
    BlockedLocally,
    NoReply,
}

/// Whether the game is writing its log. A launch without `-condebug` leaves the last launch's file
/// in place, so a file that exists proves nothing until it changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GameLog {
    #[default]
    NoGame,
    /// The game is running and the log hasn't changed since it was first seen.
    Checking,
    Writing,
    /// Nothing written for `SILENT_AFTER`. The log can go a minute without a line, so it's longer.
    Silent,
}

impl GameLog {
    /// The next state, given whether the game runs, whether the log just changed, and how long the
    /// game has been seen without a change.
    fn next(self, game: bool, wrote: bool, waited: Duration) -> Self {
        match (game, self) {
            (false, _) => Self::NoGame,
            (true, _) if wrote => Self::Writing,
            (true, Self::Writing) => Self::Writing,
            (true, _) if waited >= SILENT_AFTER => Self::Silent,
            (true, _) => Self::Checking,
        }
    }
}

/// What the counters show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Monitor {
    pub direct: Direct,
    pub exitlag: bool,
    /// A game from `diag::GAMES` is running.
    pub game: bool,
    pub log_found: bool,
    /// Whether the running game is writing its log, which a match hold depends on.
    pub game_log: GameLog,
    pub route: Option<Route>,
    /// The game's last `Ping location` table, empty until it has measured.
    pub table: Vec<PingEntry>,
}

/// How a chip's value is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Good,
    Warn,
}

/// One status chip: a name, a short value, and the explanation behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub name: &'static str,
    pub value: String,
    pub tip: String,
    pub tone: Tone,
}

impl Chip {
    fn new(name: &'static str, value: impl Into<String>, tip: impl Into<String>) -> Self {
        Self {
            name,
            value: value.into(),
            tip: tip.into(),
            tone: Tone::Plain,
        }
    }

    fn tone(self, tone: Tone) -> Self {
        Self { tone, ..self }
    }

    /// The chip as one line, for the console and the log.
    pub fn line(&self) -> String {
        format!("{}: {}. {}", self.name, self.value, self.tip)
    }
}

pub const HOLD: &str = "Connection Hold";

/// The site of the match the game is in, if the log shows one. A pick that would cut it waits.
/// A log the game isn't writing could hold a match from an earlier launch, so it doesn't count.
pub fn match_site(monitor: &Monitor) -> Option<&str> {
    let route = monitor.route.as_ref()?;
    (route.state == RouteState::InMatch && monitor.game && monitor.game_log != GameLog::Silent)
        .then_some(route.site.as_str())
}

/// The direct counter. ExitLag is only mentioned when it's running.
pub fn direct_chip(pick: &str, monitor: &Monitor) -> Option<Chip> {
    let chip = |value: &str, tip: String| Some(Chip::new("Direct", value, tip));
    match monitor.direct {
        Direct::Off => chip(
            "Off",
            "Pinging is off because ping_every is 0. Change it in Settings (F2).".to_string(),
        ),
        _ if pick.is_empty() => None,
        Direct::NoPick | Direct::Waiting => {
            chip("Measuring", format!("Pinging {pick} from this PC."))
        }
        Direct::Ms(ms) if monitor.exitlag => chip(
            &format!("{ms} ms"),
            format!(
                "Ping from this PC straight to {pick}, not through ExitLag. The game's ping can \
                 differ a lot."
            ),
        ),
        Direct::Ms(ms) => chip(
            &format!("{ms} ms"),
            format!("Ping from this PC straight to {pick}. The game's own path can differ."),
        ),
        Direct::BlockedLocally => chip(
            "Blocked",
            format!("A filter on this PC stopped the ping to {pick}."),
        )
        .map(|chip| chip.tone(Tone::Warn)),
        Direct::NoReply => chip(
            "No reply",
            format!(
                "{pick} didn't answer the last few pings. Single misses are common and don't mean \
                 a block."
            ),
        )
        .map(|chip| chip.tone(Tone::Warn)),
    }
}

/// The game's own figure. It always says where it came from and when.
pub fn route_chip(pick: &str, monitor: &Monitor) -> Chip {
    let name = if monitor.exitlag { "ExitLag" } else { "Game" };
    if !monitor.log_found {
        return Chip::new(
            name,
            "No log",
            "Add -condebug to the game's launch options to see its ping. Press F1 for how.",
        );
    }
    let Some(route) = &monitor.route else {
        return Chip::new(
            name,
            "No data",
            "The game hasn't measured relays or connected yet.",
        );
    };
    let when = match route.state {
        RouteState::InMatch => format!("in the match since {}", route.at),
        RouteState::InLobby => format!("in a hideout lobby since {}", route.at),
        RouteState::LastMatch => format!("in the last match, at {}", route.at),
        RouteState::Measured => format!("measured at {}", route.at),
    };
    let site = &route.site;
    let other = !pick.is_empty() && *site != pick;
    let through = if monitor.exitlag {
        " through ExitLag, if it's boosting the game"
    } else {
        ""
    };
    let mut tip = format!("The game's ping to {site}{through}, {when}.");
    if other {
        tip.push_str(&format!(" {site} isn't your pick."));
    }
    tip.push_str(" Read from the game's log, so it isn't live.");
    let value = if other {
        format!("{} ms {site}", route.ms)
    } else {
        format!("{} ms", route.ms)
    };
    let chip = Chip::new(name, value, tip);
    if other { chip.tone(Tone::Warn) } else { chip }
}

/// What a switch still needs, or `None` when there's nothing to say. The game measures relays only
/// at startup, so the pick reaches matchmaking only once a measurement has seen the block.
pub fn hold_chip(
    pick: &str,
    pending: Option<&str>,
    view: GameView,
    monitor: &Monitor,
) -> Option<Chip> {
    let chip = |value: &str, tip: String| Some(Chip::new(HOLD, value, tip));
    if let Some(pending) = pending {
        return chip(
            &format!("{pending} after match"),
            format!(
                "You picked {pending} during a match, so it waits for the match to end instead of \
                 cutting it. 'clear pending' cancels it."
            ),
        )
        .map(|chip| chip.tone(Tone::Warn));
    }
    if monitor.game && (!monitor.log_found || monitor.game_log == GameLog::Silent) {
        return chip(
            "No log",
            "The game isn't writing console.log, so a match can't be seen and a pick applies at \
             once, even mid-match. Add -condebug to the game's launch options (F1), then restart \
             the game."
                .to_string(),
        )
        .map(|chip| chip.tone(Tone::Warn));
    }
    if let Some(site) = match_site(monitor) {
        return chip(
            "In match",
            format!(
                "In a match on {site}. A pick that closes {site} waits for the match to end. \
                 'set pick <server> --now' doesn't wait."
            ),
        )
        .map(|chip| chip.tone(Tone::Good));
    }
    if !monitor.log_found {
        return chip(
            "No log",
            "Add -condebug to the game's launch options (F1). Without its log, a pick can't wait \
             for a match to end, and this can't say when the game needs a restart."
                .to_string(),
        );
    }
    if monitor.game_log == GameLog::Checking {
        return chip(
            "Checking",
            "Waiting for the game to write its log.".to_string(),
        );
    }
    if pick.is_empty() {
        return None;
    }
    let stay_open = if monitor.exitlag {
        "Steam and ExitLag can stay open."
    } else {
        "Steam can stay open."
    };
    match view {
        GameView::Unknown if monitor.game => chip(
            "Measuring",
            "Waiting for the game to measure relays.".to_string(),
        ),
        GameView::Unknown => None,
        GameView::Ready { ms } => chip(
            "Ready",
            format!("The game measured {pick} at {ms} ms and no blocked server."),
        )
        .map(|chip| chip.tone(Tone::Good)),
        GameView::Stale if monitor.game => chip(
            "Restart game",
            format!(
                "The game measured relays before {pick} was picked, and only measures when it \
                 starts. Restart the game. {stay_open}"
            ),
        )
        .map(|chip| chip.tone(Tone::Warn)),
        GameView::Stale => chip(
            "Next launch",
            format!("The game will measure {pick} when it starts."),
        ),
    }
}

/// Turns ping results into what the counter shows.
#[derive(Debug, Default)]
struct DirectTracker {
    best: Option<Ipv4Addr>,
    last: Option<u32>,
    misses: u32,
}

impl DirectTracker {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn hit(&mut self, ip: Ipv4Addr, ms: u32) -> Direct {
        self.best = Some(ip);
        self.last = Some(ms);
        self.misses = 0;
        Direct::Ms(ms)
    }

    fn miss(&mut self, code: u32) -> Direct {
        self.misses += 1;
        if let Some(ms) = self.last
            && self.misses <= HOLD_MISSES
        {
            return Direct::Ms(ms);
        }
        self.last = None;
        if code == diag::IP_GENERAL_FAILURE {
            Direct::BlockedLocally
        } else {
            Direct::NoReply
        }
    }

    /// Pings the best known relay, and sweeps all of them only on a miss.
    fn measure(&mut self, ips: &[Ipv4Addr]) -> Direct {
        let mut failure = 0;
        if let Some(ip) = self.best.filter(|ip| ips.contains(ip)) {
            match diag::ping(ip) {
                Ok(ms) => return self.hit(ip, ms),
                Err(code) => failure = code,
            }
        }
        for (rtt, ip) in diag::probe(ips) {
            match rtt {
                Ok(ms) => return self.hit(ip, ms),
                Err(code) => failure = code,
            }
        }
        self.miss(failure)
    }
}

/// Reads only what's new in the log, and starts again when the file is rewritten.
#[derive(Default)]
struct LogTail {
    configured: Option<String>,
    path: Option<PathBuf>,
    next_search: Option<Instant>,
    /// The file's size and time when last read, so an unchanged log isn't opened.
    seen: Option<(u64, SystemTime)>,
    offset: u64,
    head: Vec<u8>,
    partial: Vec<u8>,
    route: LiveRoute,
    /// The file changed since it was last read, not counting a first read.
    wrote: bool,
}

impl LogTail {
    fn restart(&mut self) {
        self.seen = None;
        self.offset = 0;
        self.head.clear();
        self.partial.clear();
        self.route = LiveRoute::default();
    }

    /// Takes the file's current length and first bytes, and returns where to read from.
    fn seek_position(&mut self, length: u64, head: &[u8]) -> u64 {
        if length < self.offset || (!self.head.is_empty() && !head.starts_with(&self.head)) {
            self.restart();
        }
        if self.head.is_empty() {
            self.head = head.to_vec();
        }
        if self.offset == 0 {
            self.offset = length.saturating_sub(MAX_FIRST_READ);
            if self.offset > 0 {
                // The first line would be cut in half, so skip it.
                self.partial.clear();
            }
        }
        self.offset
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.offset += bytes.len() as u64;
        self.partial.extend_from_slice(bytes);
        let Some(end) = self.partial.iter().rposition(|byte| *byte == b'\n') else {
            return;
        };
        let complete: Vec<u8> = self.partial.drain(..=end).collect();
        for line in complete.split(|byte| *byte == b'\n') {
            self.route.feed(&String::from_utf8_lossy(line));
        }
    }

    /// Whether the file changed since this was last asked.
    fn take_wrote(&mut self) -> bool {
        std::mem::take(&mut self.wrote)
    }

    /// Reads new lines. Returns whether a log was found.
    fn poll(&mut self, configured: Option<&str>) -> bool {
        let same_setting = self.configured.as_deref() == configured;
        let known = self
            .path
            .clone()
            .filter(|path| same_setting && path.is_file());
        let path = match known {
            Some(path) => path,
            None => {
                let now = Instant::now();
                if same_setting && self.next_search.is_some_and(|next| now < next) {
                    return false;
                }
                self.next_search = Some(now + SEARCH_EVERY);
                self.configured = configured.map(str::to_string);
                let Ok(path) = diag::find_log(configured) else {
                    self.path = None;
                    self.restart();
                    return false;
                };
                path
            }
        };
        if self.path.as_ref() != Some(&path) {
            self.path = Some(path.clone());
            self.restart();
        }
        self.read_new(&path).is_ok()
    }

    fn read_new(&mut self, path: &Path) -> std::io::Result<()> {
        let metadata = fs::metadata(path)?;
        let now_seen = Some((metadata.len(), metadata.modified()?));
        if self.seen == now_seen {
            return Ok(());
        }
        self.wrote |= self.seen.is_some();
        self.seen = now_seen;
        let mut file: File = diag::open_shared(path)?;
        let length = metadata.len();
        let mut head = vec![0; HEAD_BYTES];
        let got = file.read(&mut head)?;
        head.truncate(got);
        let position = self.seek_position(length, &head);
        file.seek(SeekFrom::Start(position))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        self.feed(&bytes);
        Ok(())
    }
}

/// What changes what the monitor does.
pub enum Control {
    Targets { pick: String, ips: Vec<Ipv4Addr> },
    PingEvery(u32),
    LogPath(Option<String>),
}

/// Starts the monitor. `report` receives each change and returns false once the app is closing.
pub fn spawn(
    ping_every: u32,
    log_path: Option<String>,
    report: impl Fn(Monitor) -> bool + Send + 'static,
) -> Sender<Control> {
    let (control, receiver) = channel();
    thread::spawn(move || run(ping_every, log_path, &receiver, report));
    control
}

fn run(
    mut ping_every: u32,
    mut log_path: Option<String>,
    control: &Receiver<Control>,
    report: impl Fn(Monitor) -> bool,
) {
    let mut pick = String::new();
    let mut ips: Vec<Ipv4Addr> = Vec::new();
    let mut tracker = DirectTracker::default();
    let mut tail = LogTail::default();
    let mut state = Monitor::default();
    let mut next_ping = Instant::now();
    let mut next_log = Instant::now();
    let mut game_since: Option<Instant> = None;
    loop {
        let now = Instant::now();
        if now >= next_log {
            (state.exitlag, state.game) = diag::exitlag_and_game_running();
            state.log_found = tail.poll(log_path.as_deref());
            let wrote = tail.take_wrote();
            game_since = state.game.then(|| game_since.unwrap_or(now));
            let waited = game_since.map_or(Duration::ZERO, |since| now - since);
            state.game_log = state.game_log.next(state.game, wrote, waited);
            state.route = tail.route.current(&pick);
            state.table = tail.route.table().to_vec();
            next_log = now + LOG_EVERY;
        }
        if ping_every == 0 {
            state.direct = Direct::Off;
        } else if ips.is_empty() {
            state.direct = Direct::NoPick;
        } else if now >= next_ping {
            state.direct = tracker.measure(&ips);
            next_ping = Instant::now() + Duration::from_secs(u64::from(ping_every));
        }
        if !report(state.clone()) {
            return;
        }
        let wake = if ping_every == 0 || ips.is_empty() {
            next_log
        } else {
            next_log.min(next_ping)
        };
        match control.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(Control::Targets {
                pick: new_pick,
                ips: new_ips,
            }) => {
                if new_pick != pick {
                    tracker.reset();
                    state.direct = Direct::Waiting;
                    next_ping = Instant::now();
                    next_log = Instant::now();
                }
                pick = new_pick;
                ips = new_ips;
            }
            Ok(Control::PingEvery(every)) => {
                ping_every = every;
                next_ping = Instant::now();
            }
            Ok(Control::LogPath(path)) => {
                log_path = path;
                next_log = Instant::now();
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IP: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
    const TIMED_OUT: u32 = 11010;

    fn route(state: RouteState, site: &str) -> Route {
        Route {
            site: site.to_string(),
            ms: 121,
            at: "18:46".to_string(),
            state,
        }
    }

    fn running(route: Option<Route>) -> Monitor {
        Monitor {
            exitlag: true,
            log_found: true,
            route,
            ..Monitor::default()
        }
    }

    #[test]
    fn a_value_holds_for_two_misses_then_clears() {
        let mut tracker = DirectTracker::default();
        assert_eq!(tracker.hit(IP, 100), Direct::Ms(100));
        assert_eq!(tracker.miss(TIMED_OUT), Direct::Ms(100));
        assert_eq!(tracker.miss(TIMED_OUT), Direct::Ms(100));
        assert_eq!(tracker.miss(TIMED_OUT), Direct::NoReply);
        assert_eq!(tracker.miss(TIMED_OUT), Direct::NoReply);
    }

    #[test]
    fn a_reply_resets_the_miss_count() {
        let mut tracker = DirectTracker::default();
        tracker.hit(IP, 100);
        tracker.miss(TIMED_OUT);
        tracker.miss(TIMED_OUT);
        assert_eq!(tracker.hit(IP, 90), Direct::Ms(90));
        assert_eq!(tracker.miss(TIMED_OUT), Direct::Ms(90));
    }

    #[test]
    fn a_first_miss_has_nothing_to_hold() {
        let mut tracker = DirectTracker::default();
        assert_eq!(tracker.miss(TIMED_OUT), Direct::NoReply);
    }

    #[test]
    fn a_local_filter_is_named() {
        let mut tracker = DirectTracker::default();
        assert_eq!(
            tracker.miss(diag::IP_GENERAL_FAILURE),
            Direct::BlockedLocally
        );
    }

    fn direct(direct: Direct) -> Monitor {
        Monitor {
            direct,
            ..Monitor::default()
        }
    }

    fn direct_value(pick: &str, monitor: &Monitor) -> Option<String> {
        direct_chip(pick, monitor).map(|chip| chip.value)
    }

    #[test]
    fn direct_chips_say_what_they_measure() {
        let chip = direct_chip("sgp", &direct(Direct::Ms(356))).unwrap();
        assert_eq!(chip.value, "356 ms");
        assert!(chip.tip.contains("straight to sgp"), "{}", chip.tip);
        assert_eq!(direct_value("", &direct(Direct::NoPick)), None);
        assert_eq!(direct_value("sgp", &direct(Direct::Off)).unwrap(), "Off");
        assert_eq!(
            direct_value("sgp", &direct(Direct::NoReply)).unwrap(),
            "No reply"
        );
        let blocked = direct_chip("sgp", &direct(Direct::BlockedLocally)).unwrap();
        assert_eq!(
            (blocked.value.as_str(), blocked.tone),
            ("Blocked", Tone::Warn)
        );
        assert!(blocked.tip.contains("filter"));
        assert_eq!(
            direct_value("sgp", &direct(Direct::Waiting)).unwrap(),
            "Measuring"
        );
        assert_eq!(
            direct_value("", &direct(Direct::NoReply)),
            None,
            "a figure from before the block was cleared isn't shown"
        );
    }

    #[test]
    fn exitlag_is_only_mentioned_while_it_runs() {
        let with = Monitor {
            exitlag: true,
            ..direct(Direct::Ms(356))
        };
        assert!(
            direct_chip("sgp", &with)
                .unwrap()
                .tip
                .contains("not through ExitLag")
        );
        assert_eq!(route_chip("sgp", &with).name, "ExitLag");
        for monitor in [
            Monitor::default(),
            direct(Direct::Ms(356)),
            Monitor {
                log_found: true,
                game: true,
                route: Some(route(RouteState::InMatch, "sgp")),
                ..direct(Direct::Ms(356))
            },
        ] {
            let chips = [
                direct_chip("sgp", &monitor),
                Some(route_chip("sgp", &monitor)),
                hold_chip("sgp", None, GameView::Stale, &monitor),
            ];
            for chip in chips.into_iter().flatten() {
                assert!(!chip.line().contains("ExitLag"), "{}", chip.line());
            }
        }
    }

    #[test]
    fn route_chips_give_the_figure_and_say_where_it_came_from() {
        let game = Monitor {
            exitlag: false,
            ..running(Some(route(RouteState::InMatch, "sgp")))
        };
        let chip = route_chip("sgp", &game);
        assert_eq!((chip.name, chip.value.as_str()), ("Game", "121 ms"));
        assert!(
            chip.tip.contains("in the match since 18:46"),
            "{}",
            chip.tip
        );
        assert!(chip.tip.contains("isn't live"));
        for (state, when) in [
            (RouteState::Measured, "measured at 18:46"),
            (RouteState::LastMatch, "last match, at 18:46"),
            (RouteState::InLobby, "hideout lobby since 18:46"),
        ] {
            let chip = route_chip("sgp", &running(Some(route(state, "sgp"))));
            assert!(chip.tip.contains(when), "{}", chip.tip);
        }
    }

    #[test]
    fn a_route_to_another_site_says_so() {
        let chip = route_chip("fra", &running(Some(route(RouteState::InMatch, "sgp"))));
        assert_eq!((chip.value.as_str(), chip.tone), ("121 ms sgp", Tone::Warn));
        assert!(chip.tip.contains("sgp isn't your pick"), "{}", chip.tip);
        let chip = route_chip("", &running(Some(route(RouteState::InMatch, "sgp"))));
        assert_eq!((chip.value.as_str(), chip.tone), ("121 ms", Tone::Plain));
    }

    #[test]
    fn route_chips_cover_having_nothing_to_show() {
        assert_eq!(route_chip("sgp", &Monitor::default()).value, "No log");
        assert_eq!(route_chip("sgp", &running(None)).value, "No data");
    }

    fn hold(
        pick: &str,
        pending: Option<&str>,
        view: GameView,
        monitor: &Monitor,
    ) -> Option<String> {
        hold_chip(pick, pending, view, monitor).map(|chip| chip.value)
    }

    #[test]
    fn hold_chips_say_what_is_left_to_do() {
        let game = Monitor {
            game: true,
            game_log: GameLog::Writing,
            ..running(None)
        };
        let closed = running(None);
        let value = |pending, view, monitor: &Monitor| hold("hkg", pending, view, monitor);
        assert_eq!(
            value(Some("tyo"), GameView::Stale, &game).unwrap(),
            "tyo after match"
        );
        assert_eq!(value(None, GameView::Stale, &game).unwrap(), "Restart game");
        assert_eq!(
            value(None, GameView::Stale, &closed).unwrap(),
            "Next launch"
        );
        let ready = hold_chip("hkg", None, GameView::Ready { ms: 152 }, &game).unwrap();
        assert_eq!((ready.value.as_str(), ready.tone), ("Ready", Tone::Good));
        assert!(ready.tip.contains("measured hkg at 152 ms"));
        assert_eq!(value(None, GameView::Unknown, &game).unwrap(), "Measuring");
        assert_eq!(value(None, GameView::Unknown, &closed), None);
        assert_eq!(hold("", None, GameView::Stale, &game), None);
        assert_eq!(
            hold_chip("hkg", None, GameView::Stale, &game).unwrap().name,
            "Connection Hold"
        );
        for view in [
            GameView::Unknown,
            GameView::Stale,
            GameView::Ready { ms: 1 },
        ] {
            for monitor in [&game, &closed] {
                if let Some(chip) = hold_chip("hkg", None, view, monitor) {
                    assert!(chip.line().is_ascii(), "{}", chip.line());
                }
            }
        }
    }

    #[test]
    fn a_match_shows_on_the_hold_chip_whatever_the_pick() {
        let playing = Monitor {
            game: true,
            game_log: GameLog::Writing,
            ..running(Some(route(RouteState::InMatch, "sgp")))
        };
        for pick in ["sgp", "hkg", ""] {
            let chip = hold_chip(pick, None, GameView::Ready { ms: 1 }, &playing).unwrap();
            assert_eq!(chip.value, "In match", "{pick}");
            assert!(chip.tip.contains("closes sgp waits"));
        }
        let after = Monitor {
            game: false,
            game_log: GameLog::NoGame,
            ..playing.clone()
        };
        assert_eq!(
            hold("sgp", None, GameView::Ready { ms: 1 }, &after).unwrap(),
            "Ready",
            "a closed game has no match to protect"
        );
        assert_eq!(
            hold("sgp", Some("tyo"), GameView::Ready { ms: 1 }, &playing).unwrap(),
            "tyo after match",
            "a waiting pick says so first"
        );
    }

    #[test]
    fn a_game_that_isnt_logging_turns_the_hold_off() {
        let silent = Monitor {
            game: true,
            game_log: GameLog::Silent,
            ..running(Some(route(RouteState::InMatch, "sgp")))
        };
        assert_eq!(
            match_site(&silent),
            None,
            "an old launch's match doesn't count"
        );
        let chip = hold_chip("sgp", None, GameView::Ready { ms: 1 }, &silent).unwrap();
        assert_eq!((chip.value.as_str(), chip.tone), ("No log", Tone::Warn));
        assert!(chip.tip.contains("-condebug"));
        let missing = Monitor {
            game: true,
            ..Monitor::default()
        };
        assert_eq!(
            hold("sgp", None, GameView::Unknown, &missing).unwrap(),
            "No log"
        );
        let checking = Monitor {
            game_log: GameLog::Checking,
            ..silent.clone()
        };
        assert_eq!(
            match_site(&checking),
            Some("sgp"),
            "a match is protected while the log is still being checked"
        );
        let idle = Monitor {
            route: None,
            ..checking
        };
        assert_eq!(
            hold("sgp", None, GameView::Ready { ms: 1 }, &idle).unwrap(),
            "Checking"
        );
        assert_eq!(
            hold("sgp", None, GameView::Unknown, &Monitor::default()).unwrap(),
            "No log",
            "with the game closed, it still says logging is off"
        );
    }

    #[test]
    fn the_log_counts_as_written_once_it_changes() {
        let minute = Duration::from_secs(60);
        let start = GameLog::NoGame;
        assert_eq!(start.next(false, true, minute), GameLog::NoGame);
        assert_eq!(start.next(true, false, Duration::ZERO), GameLog::Checking);
        assert_eq!(
            GameLog::Checking.next(true, false, SILENT_AFTER),
            GameLog::Silent
        );
        assert_eq!(GameLog::Silent.next(true, true, minute), GameLog::Writing);
        assert_eq!(
            GameLog::Writing.next(true, false, SILENT_AFTER * 10),
            GameLog::Writing,
            "a logging game can go quiet for a long time"
        );
        assert_eq!(GameLog::Writing.next(false, false, minute), GameLog::NoGame);
    }

    #[test]
    fn a_first_read_isnt_a_write() {
        let mut tail = LogTail::default();
        let dir = std::env::temp_dir().join(format!("dl-conn-rs-tail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("console.log");
        fs::write(&path, LOCATION).unwrap();
        tail.read_new(&path).unwrap();
        assert!(!tail.take_wrote());
        let mut more = LOCATION.to_vec();
        more.extend_from_slice(LOCATION);
        fs::write(&path, &more).unwrap();
        tail.read_new(&path).unwrap();
        assert!(tail.take_wrote());
        assert!(!tail.take_wrote(), "taking it clears it");
        fs::remove_dir_all(&dir).unwrap();
    }

    const LOCATION: &[u8] = b"11/05 18:46:31 [SteamNetSockets] Ping location: sgp=120+12/121+12\n";

    #[test]
    fn the_tail_only_reads_complete_lines() {
        let mut tail = LogTail::default();
        let (start, end) = LOCATION.split_at(30);
        tail.feed(start);
        assert_eq!(tail.route.current("sgp"), None);
        tail.feed(end);
        assert_eq!(tail.route.current("sgp").unwrap().ms, 120);
        assert_eq!(tail.offset, LOCATION.len() as u64);
    }

    #[test]
    fn the_tail_starts_again_when_the_file_shrinks() {
        let mut tail = LogTail::default();
        let head = &LOCATION[..HEAD_BYTES];
        assert_eq!(tail.seek_position(LOCATION.len() as u64, head), 0);
        tail.feed(LOCATION);
        assert_eq!(tail.seek_position(10, head), 0);
        assert_eq!(tail.route.current("sgp"), None, "old state is gone");
    }

    #[test]
    fn the_tail_starts_again_when_the_file_is_rewritten_larger() {
        let mut tail = LogTail::default();
        let head = &LOCATION[..HEAD_BYTES];
        tail.seek_position(LOCATION.len() as u64, head);
        tail.feed(LOCATION);
        let other = vec![b'x'; HEAD_BYTES];
        assert_eq!(tail.seek_position(1000, &other), 0);
        assert_eq!(tail.route.current("sgp"), None);
    }

    #[test]
    fn the_tail_keeps_its_place_as_the_file_grows() {
        let mut tail = LogTail::default();
        let head = &LOCATION[..HEAD_BYTES];
        tail.seek_position(LOCATION.len() as u64, head);
        tail.feed(LOCATION);
        let offset = LOCATION.len() as u64;
        assert_eq!(tail.seek_position(offset + 500, head), offset);
        assert!(tail.route.current("sgp").is_some());
    }

    #[test]
    fn the_tail_skips_most_of_a_huge_log() {
        let mut tail = LogTail::default();
        let length = MAX_FIRST_READ + 1000;
        assert_eq!(tail.seek_position(length, b"head"), 1000);
    }
}
