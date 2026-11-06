//! The two ping counters: direct pings to the pick, and the game's own figure from its log.
//!
//! This runs on its own thread so a slow ping never holds up commands. The direct figure is a real
//! measurement of this PC's path. The game's figure is the last one it wrote, and it's the only one
//! that sees a routing tool such as ExitLag, if one is in use. Nothing here needs ExitLag or Steam.

use crate::analysis::{LiveRoute, Route, RouteState};
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

/// What the counters show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Monitor {
    pub direct: Direct,
    pub exitlag: bool,
    pub log_found: bool,
    pub route: Option<Route>,
}

/// The direct counter's text. ExitLag is only mentioned when it's running.
pub fn direct_label(pick: &str, monitor: &Monitor) -> String {
    match monitor.direct {
        Direct::Off => "Direct: off, ping interval is 0".to_string(),
        _ if pick.is_empty() => "Direct: pick a server".to_string(),
        Direct::NoPick | Direct::Waiting => format!("Direct: measuring {pick}"),
        Direct::Ms(ms) if monitor.exitlag => {
            format!("Direct: {ms} ms to {pick} (this PC, not through ExitLag)")
        }
        Direct::Ms(ms) => format!("Direct: {ms} ms to {pick} (ping from this PC)"),
        Direct::BlockedLocally => format!("Direct: {pick} blocked by a filter on this PC"),
        Direct::NoReply => format!("Direct: no reply from {pick}"),
    }
}

/// The game's figure as text. It always says where it came from and when.
pub fn route_label(pick: &str, monitor: &Monitor) -> String {
    let name = if monitor.exitlag {
        "ExitLag route"
    } else {
        "Game"
    };
    if !monitor.log_found {
        return format!("{name}: turn on logging to see this, press F1");
    }
    let Some(route) = &monitor.route else {
        return format!("{name}: no game data yet, press F1");
    };
    let when = match route.state {
        RouteState::InMatch => "in match",
        RouteState::LastMatch => "last match",
        RouteState::Measured => "measured",
    };
    let other = if !pick.is_empty() && route.site != pick {
        ", not your pick"
    } else {
        ""
    };
    format!(
        "{name}: {} ms to {}{other}, {when} at {} (game log, not live)",
        route.ms, route.site, route.at
    )
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
    loop {
        let now = Instant::now();
        if now >= next_log {
            state.exitlag = diag::exitlag_running();
            state.log_found = tail.poll(log_path.as_deref());
            state.route = tail.route.current(&pick);
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

    #[test]
    fn direct_labels_say_what_they_measure() {
        assert_eq!(
            direct_label("sgp", &direct(Direct::Ms(356))),
            "Direct: 356 ms to sgp (ping from this PC)"
        );
        assert_eq!(
            direct_label("", &direct(Direct::NoPick)),
            "Direct: pick a server"
        );
        assert!(direct_label("sgp", &direct(Direct::Off)).contains("off"));
        assert!(direct_label("sgp", &direct(Direct::NoReply)).contains("no reply from sgp"));
        assert!(direct_label("sgp", &direct(Direct::BlockedLocally)).contains("filter"));
        assert!(direct_label("sgp", &direct(Direct::Waiting)).contains("measuring"));
        assert!(direct_label("sgp", &direct(Direct::NoPick)).contains("measuring"));
        assert_eq!(
            direct_label("", &direct(Direct::NoReply)),
            "Direct: pick a server",
            "a figure from before the block was cleared isn't shown"
        );
    }

    #[test]
    fn exitlag_is_only_mentioned_while_it_runs() {
        let with = Monitor {
            exitlag: true,
            ..direct(Direct::Ms(356))
        };
        assert_eq!(
            direct_label("sgp", &with),
            "Direct: 356 ms to sgp (this PC, not through ExitLag)"
        );
        for monitor in [
            Monitor::default(),
            direct(Direct::Ms(356)),
            Monitor {
                log_found: true,
                route: Some(route(RouteState::InMatch, "sgp")),
                ..direct(Direct::Ms(356))
            },
        ] {
            assert!(!direct_label("sgp", &monitor).contains("ExitLag"));
            assert!(!route_label("sgp", &monitor).contains("ExitLag"));
        }
    }

    #[test]
    fn the_game_figure_shows_without_exitlag() {
        let monitor = Monitor {
            exitlag: false,
            ..running(Some(route(RouteState::InMatch, "sgp")))
        };
        assert_eq!(
            route_label("sgp", &monitor),
            "Game: 121 ms to sgp, in match at 18:46 (game log, not live)"
        );
    }

    #[test]
    fn route_labels_give_their_source_and_time() {
        assert_eq!(
            route_label("sgp", &running(Some(route(RouteState::InMatch, "sgp")))),
            "ExitLag route: 121 ms to sgp, in match at 18:46 (game log, not live)"
        );
        assert_eq!(
            route_label("sgp", &running(Some(route(RouteState::Measured, "sgp")))),
            "ExitLag route: 121 ms to sgp, measured at 18:46 (game log, not live)"
        );
        assert_eq!(
            route_label("sgp", &running(Some(route(RouteState::LastMatch, "sgp")))),
            "ExitLag route: 121 ms to sgp, last match at 18:46 (game log, not live)"
        );
    }

    #[test]
    fn a_route_to_another_site_says_so() {
        let label = route_label("fra", &running(Some(route(RouteState::InMatch, "sgp"))));
        assert!(label.contains("to sgp, not your pick"), "{label}");
        let label = route_label("", &running(Some(route(RouteState::InMatch, "sgp"))));
        assert!(!label.contains("not your pick"), "{label}");
    }

    #[test]
    fn labels_cover_having_nothing_to_show() {
        assert_eq!(
            route_label("sgp", &Monitor::default()),
            "Game: turn on logging to see this, press F1"
        );
        assert_eq!(
            route_label(
                "sgp",
                &Monitor {
                    exitlag: true,
                    ..Monitor::default()
                }
            ),
            "ExitLag route: turn on logging to see this, press F1"
        );
        assert_eq!(
            route_label("sgp", &running(None)),
            "ExitLag route: no game data yet, press F1"
        );
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
