//! Reads the game's `console.log` and turns it into a report. Pure: file access is in `diag`.
//!
//! Steam IDs, player names and launch options other than the two logging flags never leave this
//! module. Only `SDR` connections count as relay connections. Pipe and loopback connections are the
//! local hideout server.

use crate::regions;

/// Whether a connection has ended, for the route counter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RouteState {
    InMatch,
    LastMatch,
    Measured,
}

/// The game's own latency figure. It's the last one logged, not a live reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub site: String,
    pub ms: u32,
    /// `HH:MM` as logged.
    pub at: String,
    pub state: RouteState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Measure {
    pub at: String,
    pub secs: String,
    pub valid: u32,
    pub great: u32,
    pub good: u32,
    pub ok: u32,
    pub ignored: u32,
}

/// One site from a `Ping location` line, such as `sgp=120+12/121+12`.
#[derive(Debug, Clone, PartialEq)]
pub struct PingEntry {
    pub site: String,
    /// `None` when the game has no direct measurement, as for a blocked site.
    pub direct: Option<u32>,
    pub via: Option<u32>,
    pub penalty: u32,
}

impl PingEntry {
    fn best(&self) -> Option<u32> {
        self.direct.or(self.via)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selected {
    pub at: String,
    pub relay: String,
    pub address: String,
    pub ping: u32,
    pub breakdown: String,
    pub rank: String,
    pub score: String,
    /// `None` for the primary, otherwise the backup's number.
    pub backup: Option<u32>,
}

impl Selected {
    fn site(&self) -> &str {
        site_of(&self.relay)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lost {
    pub at: String,
    pub role: String,
    pub relay: String,
    pub reason: String,
}

/// The game's closing statistics for a connection, from this PC's side unless named otherwise.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub data_center: Option<String>,
    pub sent: Option<String>,
    pub received: Option<String>,
    pub dropped: Option<(String, String)>,
    pub out_of_order: Option<(String, String)>,
    pub remote_dropped: Option<(String, String)>,
    /// 5th, 50th, 75th, 95th and 98th percentile, in ms.
    pub ping: Vec<u32>,
    /// 50th, 25th, 5th and 2nd percentile, in percent.
    pub quality: Vec<u32>,
    pub steady: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Connection {
    pub id: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub connected: bool,
    pub match_id: Option<String>,
    pub selected: Vec<Selected>,
    pub lost: Vec<Lost>,
    pub events: Vec<(String, String)>,
    pub end_reason: Option<String>,
    pub summary: Summary,
}

impl Connection {
    fn primary(&self) -> Option<&Selected> {
        self.selected.iter().rfind(|pick| pick.backup.is_none())
    }

    fn backups(&self) -> impl Iterator<Item = &Selected> {
        self.selected.iter().filter(|pick| pick.backup.is_some())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Log {
    pub first: Option<String>,
    pub last: Option<String>,
    pub condebug: bool,
    pub lognetapi: bool,
    pub saw_options: bool,
    pub revision: Option<String>,
    pub fetch_failures: Vec<(String, String)>,
    pub offline: Vec<(String, String, String)>,
    pub network: Option<String>,
    pub measures: Vec<Measure>,
    pub table: Vec<PingEntry>,
    pub table_at: Option<String>,
    pub connections: Vec<Connection>,
    pub local: u32,
    pub direct_udp: u32,
}

/// What the report compares the log against.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    pub path: String,
    pub bytes: u64,
    /// Only the end of a very large log was read.
    pub truncated: bool,
    pub age_secs: Option<u64>,
    pub pick: String,
    pub backup: Option<String>,
    pub direct_ms: Option<u32>,
    pub exitlag: bool,
}

/// Splits a line into its `MM/DD HH:MM:SS` stamp and the rest.
fn timed(line: &str) -> (Option<&str>, &str) {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
    let bytes = line.as_bytes();
    let shape = bytes.len() > 15
        && bytes[2] == b'/'
        && bytes[5] == b' '
        && bytes[8] == b':'
        && bytes[11] == b':'
        && bytes[14] == b' '
        && [0, 1, 3, 4, 6, 7, 9, 10, 12, 13]
            .iter()
            .all(|at| bytes[*at].is_ascii_digit());
    if shape {
        (Some(&line[..14]), &line[15..])
    } else {
        (None, line)
    }
}

/// The time of day from a full stamp.
fn clock(stamp: &str) -> &str {
    stamp.get(6..).unwrap_or(stamp)
}

fn minutes(stamp: &str) -> &str {
    stamp.get(6..11).unwrap_or(stamp)
}

/// Seconds within the month, for working out how long something lasted.
fn seconds(stamp: &str) -> Option<i64> {
    let part = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    Some(part(3..5)? * 86_400 + part(6..8)? * 3_600 + part(9..11)? * 60 + part(12..14)?)
}

fn span(start: &str, end: &str) -> Option<String> {
    let secs = seconds(end)? - seconds(start)?;
    (secs >= 0).then(|| {
        if secs >= 3600 {
            format!("{}h {}m", secs / 3600, secs % 3600 / 60)
        } else {
            format!("{}m {}s", secs / 60, secs % 60)
        }
    })
}

fn site_of(relay: &str) -> &str {
    relay.split('#').next().unwrap_or(relay)
}

/// The number that starts `text`.
fn leading_number(text: &str) -> Option<u32> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    text[..end].parse().ok()
}

/// The text between `before` and `after`.
fn between<'a>(text: &'a str, before: &str, after: &str) -> Option<&'a str> {
    let (_, rest) = text.split_once(before)?;
    Some(rest.split_once(after).map_or(rest, |(value, _)| value))
}

/// Parses `Selecting sgp#276 (103.10.124.121:27060) as primary.  (Ping = 121 = 120+1+0 ...`.
fn parse_selected(at: &str, body: &str) -> Option<Selected> {
    let rest = body.strip_prefix("Selecting ")?;
    let (relay, rest) = rest.split_once(" (")?;
    let (address, rest) = rest.split_once(") as ")?;
    let backup = rest
        .strip_prefix("backup #")
        .map(|number| leading_number(number).unwrap_or(0));
    if backup.is_none() && !rest.starts_with("primary") {
        return None;
    }
    Some(Selected {
        at: at.to_string(),
        relay: relay.to_string(),
        address: address.to_string(),
        ping: leading_number(between(rest, "Ping = ", " ")?)?,
        breakdown: between(rest, "Ping = ", " (")?
            .split(" = ")
            .nth(1)
            .unwrap_or_default()
            .to_string(),
        rank: between(rest, "Rank=", " ").unwrap_or_default().to_string(),
        score: between(rest, "Score=", ")").unwrap_or_default().to_string(),
        backup,
    })
}

/// Parses `Lost backup #1 maa2#102 (155.133.225.19:27052).  consecutive_ping_timeout`.
fn parse_lost(at: &str, body: &str) -> Option<Lost> {
    let rest = body.strip_prefix("Lost ")?;
    let (left, right) = rest.split_once(" (")?;
    let (role, relay) = left.rsplit_once(' ')?;
    let reason = right
        .split_once(')')
        .map_or("", |(_, reason)| reason)
        .trim_start_matches(['.', ' ']);
    Some(Lost {
        at: at.to_string(),
        role: role.to_string(),
        relay: relay.to_string(),
        reason: reason.trim().to_string(),
    })
}

/// Parses `sgp=120+12/121+12,maa2=/152+12`.
fn parse_location(text: &str) -> Vec<PingEntry> {
    text.split(',')
        .filter_map(|entry| {
            let (site, values) = entry.trim().split_once('=')?;
            let (direct, via) = values.split_once('/').unwrap_or((values, ""));
            let ms = |part: &str| part.split('+').next().and_then(|n| n.parse::<u32>().ok());
            let penalty = |part: &str| {
                part.split_once('+')
                    .and_then(|(_, penalty)| penalty.parse::<u32>().ok())
            };
            Some(PingEntry {
                site: site.to_string(),
                direct: ms(direct),
                via: ms(via),
                penalty: penalty(direct).or_else(|| penalty(via)).unwrap_or(0),
            })
        })
        .collect()
}

/// Parses `Ping measurement completed in 12.1s.  Relays: 1 valid, 0 great, 0 good+, 1 ok+, 0 ignored`.
fn parse_measure(at: &str, body: &str) -> Option<Measure> {
    let rest = body.strip_prefix("Ping measurement completed in ")?;
    let count = |name: &str| {
        rest.split([':', ',', '.'])
            .map(str::trim)
            .find_map(|part| part.strip_suffix(name)?.trim().parse().ok())
            .unwrap_or(0)
    };
    Some(Measure {
        at: at.to_string(),
        secs: rest.split("s.").next().unwrap_or_default().to_string(),
        valid: count(" valid"),
        great: count(" great"),
        good: count(" good+"),
        ok: count(" ok+"),
        ignored: count(" ignored"),
    })
}

/// A `[#id KIND ...] body` line, with everything identifying the other side dropped.
struct ConnectionLine<'a> {
    id: &'a str,
    kind: &'a str,
    label: &'a str,
    body: &'a str,
}

fn parse_connection(text: &str) -> Option<ConnectionLine<'_>> {
    let rest = text.strip_prefix("[#")?;
    // A player name in the label can contain anything, so take the last closing bracket.
    let (label, body) = rest.rsplit_once("] ")?;
    let (id, tail) = label.split_once(' ').unwrap_or((label, ""));
    let kind = tail.split(' ').next().unwrap_or(tail);
    Some(ConnectionLine {
        id,
        kind,
        label: tail,
        body,
    })
}

/// `NETWORK_DISCONNECT_DISCONNECT_BY_USER` from any of the `closed ...` forms.
fn close_reason(body: &str) -> String {
    let reason = body
        .rsplit_once(')')
        .map_or(body, |(_, reason)| reason)
        .trim_start_matches([':', ' ']);
    reason
        .strip_prefix("NETWORK_DISCONNECT_")
        .unwrap_or(reason)
        .to_string()
}

fn is_closed(body: &str) -> bool {
    body.starts_with("closed") || body.starts_with("problem detected")
}

/// Removes anything that could identify a player from text kept for the report.
fn scrub(text: &str) -> String {
    text.split_whitespace()
        .filter(|word| !word.contains("steamid:") && !word.contains("7656119"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Which part of a `Summary of connection` block is being read.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Local,
    RemoteRate,
    RemoteLifetime,
}

struct SummaryReader {
    connection: usize,
    part: Part,
    /// Lines to skip before a table's values, and which table it is.
    waiting: Option<(u32, Table)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Table {
    Ping,
    Quality,
    Variance,
}

fn counts_and_percent(text: &str) -> Option<(String, String)> {
    let mut words = text.split_whitespace();
    let count = words.next()?;
    let unit = words.next()?;
    let percent = words.next()?;
    (unit == "pkts").then(|| (count.to_string(), percent.to_string()))
}

fn table_numbers(text: &str) -> Vec<u32> {
    text.split_whitespace().filter_map(leading_number).collect()
}

impl Log {
    fn connection(&mut self, id: &str, at: Option<&str>) -> &mut Connection {
        let index = match self.connections.iter().rposition(|conn| conn.id == id) {
            Some(index) => index,
            None => {
                self.connections.push(Connection {
                    id: id.to_string(),
                    start: at.map(str::to_string),
                    ..Connection::default()
                });
                self.connections.len() - 1
            }
        };
        &mut self.connections[index]
    }

    fn networking(&mut self, text: &str, reader: &mut Option<SummaryReader>) {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix("Summary of connection [#") {
            *reader = rest.split_once(' ').and_then(|(id, tail)| {
                if !tail.starts_with("SDR") {
                    return None;
                }
                let connection = self.connections.iter().rposition(|conn| conn.id == id)?;
                Some(SummaryReader {
                    connection,
                    part: Part::Local,
                    waiting: None,
                })
            });
            return;
        }
        let Some(state) = reader else {
            return;
        };
        let summary = &mut self.connections[state.connection].summary;
        if let Some((left, table)) = state.waiting {
            if left > 0 {
                state.waiting = Some((left - 1, table));
                return;
            }
            state.waiting = None;
            match table {
                Table::Ping if summary.ping.is_empty() => summary.ping = table_numbers(text),
                Table::Quality if summary.quality.is_empty() => {
                    summary.quality = table_numbers(text);
                }
                Table::Variance if summary.steady.is_none() => {
                    summary.steady = text.split_whitespace().next().map(str::to_string);
                }
                _ => {}
            }
            return;
        }
        if let Some(rest) = text.strip_prefix("Remote host is in data center '") {
            summary.data_center = rest.split('\'').next().map(str::to_string);
        } else if text.starts_with("Rate stats received") {
            state.part = Part::RemoteRate;
        } else if text.starts_with("Lifetime stats received") {
            state.part = Part::RemoteLifetime;
        } else if state.part == Part::Local {
            if text.starts_with("Ping distribution") {
                state.waiting = Some((1, Table::Ping));
            } else if text.starts_with("Connection quality distribution") {
                state.waiting = Some((1, Table::Quality));
            } else if text.starts_with("Latency variance histogram") {
                state.waiting = Some((2, Table::Variance));
            } else if let Some(rest) = text.strip_prefix("Sent:") {
                if rest.contains("pkts ") && !rest.contains("/sec") {
                    summary.sent = rest.split_whitespace().next().map(str::to_string);
                }
            } else if let Some(rest) = text.strip_prefix("Recv:") {
                if rest.contains("pkts ") && !rest.contains("/sec") {
                    summary.received = rest.split_whitespace().next().map(str::to_string);
                }
            } else if let Some(rest) = text.strip_prefix("Dropped") {
                let rest = rest.trim_start_matches([' ', ':']);
                summary.dropped = counts_and_percent(rest);
            } else if let Some(rest) = text.strip_prefix("OutOfOrder:") {
                summary.out_of_order = counts_and_percent(rest);
            }
        } else if state.part == Part::RemoteLifetime
            && let Some(rest) = text.strip_prefix("Dropped")
        {
            let rest = rest.trim_start_matches([' ', ':']);
            summary.remote_dropped = counts_and_percent(rest);
        }
    }

    fn steam_net(&mut self, at: &str, text: &str, pending_match: &mut Option<String>) {
        if let Some(line) = parse_connection(text) {
            self.connection_line(at, &line, pending_match);
        } else if let Some(rest) = text.strip_prefix("Ping location:") {
            self.table = parse_location(rest);
            self.table_at = Some(at.to_string());
        } else if let Some(measure) = parse_measure(at, text) {
            self.measures.push(measure);
        } else if let Some(rest) = text.strip_prefix("Got SDR network config.") {
            self.revision = between(rest, "revision ", " ").map(str::to_string);
        } else if let Some(rest) = text.strip_prefix("SDR network config fetch attempt") {
            let reason = rest
                .split_once("failed.")
                .map_or("", |(_, reason)| reason)
                .split("Trying again")
                .next()
                .unwrap_or_default()
                .trim_end_matches(['.', ' '])
                .trim();
            self.fetch_failures
                .push((at.to_string(), reason.to_string()));
        } else if let Some(rest) = text.strip_prefix("SDR RelayNetworkStatus:") {
            let status = rest.split(" (").next().unwrap_or_default();
            self.network = Some(status.split_whitespace().collect::<Vec<_>>().join(" "));
        } else if let Some(rest) = text.strip_prefix("Relay ") {
            if let Some((relay, tail)) = rest.split_once(" (")
                && let Some(secs) = between(tail, "going offline in ", " ")
            {
                self.offline
                    .push((at.to_string(), relay.to_string(), secs.to_string()));
            }
        } else if let Some(rest) = text.strip_prefix("Received Steam datagram ticket") {
            *pending_match = between(rest, "match_id=", " ").map(str::to_string);
        }
    }

    fn connection_line(
        &mut self,
        at: &str,
        line: &ConnectionLine,
        pending_match: &mut Option<String>,
    ) {
        if line.kind != "SDR" {
            if line.body == "connected" {
                let loopback = line.label.contains("@127.") || line.kind == "pipe";
                if loopback {
                    self.local += 1;
                } else if line.kind == "UDP" {
                    self.direct_udp += 1;
                }
            }
            return;
        }
        let is_new = !self.connections.iter().any(|conn| conn.id == line.id);
        let conn = self.connection(line.id, Some(at));
        if is_new {
            conn.match_id = pending_match.take();
        }
        if line.body == "connected" {
            conn.connected = true;
        } else if let Some(selected) = parse_selected(at, line.body) {
            conn.selected.push(selected);
        } else if let Some(lost) = parse_lost(at, line.body) {
            conn.lost.push(lost);
        } else if is_closed(line.body) {
            conn.end = Some(at.to_string());
            conn.end_reason = Some(close_reason(line.body));
        } else if line.body.starts_with("Switch") {
            conn.events.push((at.to_string(), scrub(line.body)));
        }
    }

    fn command_line(&mut self, text: &str) {
        if text.starts_with("command line arguments") {
            return;
        }
        self.saw_options = true;
        let has = |flag: &str| text.split_whitespace().any(|word| word == flag);
        self.condebug |= has("-condebug");
        self.lognetapi |= has("-lognetapi");
    }
}

/// Reads a whole log.
pub fn parse(text: &str) -> Log {
    let mut log = Log::default();
    let mut pending_match = None;
    let mut reader = None;
    for line in text.lines() {
        let (stamp, rest) = timed(line);
        let Some(stamp) = stamp else {
            continue;
        };
        if log.first.is_none() {
            log.first = Some(stamp.to_string());
        }
        log.last = Some(stamp.to_string());
        if let Some(text) = rest.strip_prefix("[SteamNetSockets] ") {
            log.steam_net(stamp, text, &mut pending_match);
        } else if let Some(text) = rest.strip_prefix("[Networking]") {
            log.networking(text, &mut reader);
        } else if let Some(text) = rest.strip_prefix("[CommandLine] ") {
            log.command_line(text);
        } else if rest.ends_with(" created")
            && let Some(id) = between(rest, " for Match ", " ")
            && pending_match.is_none()
        {
            pending_match = Some(id.to_string());
        }
    }
    log
}

/// Follows a growing log for the route counter, keeping only what the counter needs.
#[derive(Debug, Clone, Default)]
pub struct LiveRoute {
    open: Option<(String, String, u32, String)>,
    closed: Option<Route>,
    table: Vec<PingEntry>,
    table_at: String,
}

impl LiveRoute {
    pub fn feed(&mut self, line: &str) {
        let (Some(stamp), rest) = timed(line) else {
            return;
        };
        let Some(text) = rest.strip_prefix("[SteamNetSockets] ") else {
            return;
        };
        if let Some(line) = parse_connection(text) {
            if line.kind != "SDR" {
                return;
            }
            if let Some(selected) = parse_selected(stamp, line.body) {
                if selected.backup.is_none() {
                    self.open = Some((
                        line.id.to_string(),
                        selected.site().to_string(),
                        selected.ping,
                        minutes(stamp).to_string(),
                    ));
                }
            } else if is_closed(line.body)
                && self.open.as_ref().is_some_and(|open| open.0 == line.id)
                && let Some((_, site, ms, _)) = self.open.take()
            {
                self.closed = Some(Route {
                    site,
                    ms,
                    at: minutes(stamp).to_string(),
                    state: RouteState::LastMatch,
                });
            }
        } else if let Some(rest) = text.strip_prefix("Ping location:") {
            self.table = parse_location(rest);
            self.table_at = stamp.to_string();
        }
    }

    /// The best figure for the pick: the open connection, then whichever of the last measurement
    /// and the last connection is newer.
    pub fn current(&self, pick: &str) -> Option<Route> {
        if let Some((_, site, ms, at)) = &self.open {
            return Some(Route {
                site: site.clone(),
                ms: *ms,
                at: at.clone(),
                state: RouteState::InMatch,
            });
        }
        let measured = self
            .table
            .iter()
            .find(|entry| entry.site == pick)
            .and_then(|entry| {
                Some(Route {
                    site: entry.site.clone(),
                    ms: entry.best()?,
                    at: minutes(&self.table_at).to_string(),
                    state: RouteState::Measured,
                })
            });
        match (measured, &self.closed) {
            (Some(measured), Some(closed)) if closed.at >= measured.at => Some(closed.clone()),
            (Some(measured), _) => Some(measured),
            (None, closed) => closed.clone(),
        }
    }
}

/// The folders in a Steam `libraryfolders.vdf`.
pub fn libraries(vdf: &str) -> Vec<String> {
    vdf.lines()
        .filter_map(|line| {
            let mut parts = line.split('"').filter(|part| !part.trim().is_empty());
            (parts.next()? == "path").then_some(())?;
            Some(parts.next()?.replace("\\\\", "\\"))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Check,
    Ok,
    Note,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Self::Check => "CHECK",
            Self::Ok => "OK",
            Self::Note => "NOTE",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub text: String,
}

fn finding(level: Level, text: String) -> Finding {
    Finding { level, text }
}

/// Packet loss above this is worth looking into.
const LOSS_LIMIT: f32 = 1.0;
/// A gap this wide between a connection's typical and worst ping means spikes.
const SPIKE_LIMIT_MS: u32 = 30;
/// A route within this much of direct ping is the same, given measurement noise.
const SLOWER_LIMIT_MS: u32 = 5;
const STALE_SECS: u64 = 3600;

fn percent(pair: &(String, String)) -> f32 {
    pair.1.trim_end_matches('%').parse().unwrap_or(0.0)
}

fn open_sites(context: &Context) -> Vec<&str> {
    if context.pick.is_empty() {
        return Vec::new();
    }
    std::iter::once(context.pick.as_str())
        .chain(context.backup.as_deref())
        .collect()
}

fn pick_findings(log: &Log, context: &Context, found: &mut Vec<Finding>) {
    let open = open_sites(context);
    let last = log
        .connections
        .iter()
        .rfind(|conn| conn.primary().is_some());
    if open.is_empty() {
        found.push(finding(
            Level::Note,
            "No pick is active, so the log can't be compared with a block.".to_string(),
        ));
        return;
    }
    if let Some(measure) = log.measures.last() {
        let (valid, sites) = (measure.valid as usize, open.len());
        if valid == 0 {
            found.push(finding(
                Level::Check,
                format!(
                    "No relay was valid at {}. The game could reach no relay. Check that your \
                     pick has relays online and that no other tool is blocking them.",
                    clock(&measure.at)
                ),
            ));
        } else if valid > sites {
            found.push(finding(
                Level::Check,
                format!(
                    "{valid} relays were valid at {} with {sites} site(s) open, so the block \
                     wasn't on when the game measured. Pick first, then restart the game, and \
                     close any other server picker.",
                    clock(&measure.at)
                ),
            ));
        } else {
            found.push(finding(
                Level::Ok,
                format!(
                    "{valid} relay(s) valid at {} with {sites} site(s) open.",
                    clock(&measure.at)
                ),
            ));
        }
    }
    let Some(conn) = last else {
        return;
    };
    if let Some(primary) = conn.primary() {
        if primary.site() == context.pick {
            found.push(finding(
                Level::Ok,
                format!("Primary relay {} is on your pick.", primary.relay),
            ));
        } else if open.contains(&primary.site()) {
            found.push(finding(
                Level::Note,
                format!(
                    "Primary relay {} is on your backup site, not your pick.",
                    primary.relay
                ),
            ));
        } else {
            found.push(finding(
                Level::Check,
                format!(
                    "Primary relay {} is not on your pick, {}. Check that this log is from after \
                     your pick and that ExitLag routes to the same server.",
                    primary.relay, context.pick
                ),
            ));
        }
    }
    let mut backups = 0;
    for backup in conn.backups() {
        backups += 1;
        if !open.contains(&backup.site()) {
            found.push(finding(
                Level::Check,
                format!(
                    "Backup relay {} was selected at {}, outside your open sites. That's \
                     expected if you cleared the block during this match. If not, repeat the \
                     test with the block held.",
                    backup.relay,
                    clock(&backup.at)
                ),
            ));
        }
    }
    if backups == 0 && context.backup.is_none() {
        found.push(finding(
            Level::Note,
            "No backup relay, which is expected with a strict pick. Set a default backup in F2 \
             if you want a failover route."
                .to_string(),
        ));
    }
    if let (Some(primary), Some(direct)) = (conn.primary(), context.direct_ms)
        && context.exitlag
        && primary.site() == context.pick
    {
        if primary.ping > direct + SLOWER_LIMIT_MS {
            found.push(finding(
                Level::Check,
                format!(
                    "The game logged {} ms but direct ping reads {direct} ms now, so the route \
                     looks slower than direct. Try another route or server in ExitLag.",
                    primary.ping
                ),
            ));
        } else {
            found.push(finding(
                Level::Note,
                format!(
                    "The game logged {} ms and direct ping reads {direct} ms now. Direct isn't \
                     through ExitLag, and the two weren't measured at the same moment.",
                    primary.ping
                ),
            ));
        }
    }
}

fn quality_findings(log: &Log, found: &mut Vec<Finding>) {
    for conn in &log.connections {
        for lost in &conn.lost {
            let reason = if lost.reason.is_empty() {
                String::new()
            } else {
                format!(": {}", lost.reason)
            };
            found.push(finding(
                Level::Note,
                format!(
                    "Lost {} {} at {}{reason}.",
                    lost.role,
                    lost.relay,
                    clock(&lost.at)
                ),
            ));
        }
        let summary = &conn.summary;
        if let Some(dropped) = &summary.dropped {
            if percent(dropped) > LOSS_LIMIT {
                found.push(finding(
                    Level::Check,
                    format!("This PC dropped {} of received packets.", dropped.1),
                ));
            } else {
                found.push(finding(
                    Level::Ok,
                    format!("Packet loss was low: {} dropped on this PC.", dropped.1),
                ));
            }
        }
        if let [_, typical, _, _, worst] = summary.ping[..]
            && worst > typical + SPIKE_LIMIT_MS
        {
            found.push(finding(
                Level::Check,
                format!("Ping spiked: {typical} ms typical, {worst} ms at the 98th percentile."),
            ));
        }
    }
}

/// What the log shows, worst first.
pub fn findings(log: &Log, context: &Context) -> Vec<Finding> {
    let mut found = Vec::new();
    pick_findings(log, context, &mut found);
    quality_findings(log, &mut found);
    if log.connections.is_empty() {
        found.push(finding(
            Level::Note,
            "No SDR connection in this log, only local ones. It doesn't test the route. Join a \
             custom game or match and stay connected for a couple of minutes."
                .to_string(),
        ));
    }
    if log.direct_udp > 0 {
        found.push(finding(
            Level::Note,
            format!(
                "{} direct UDP connection(s) to a remote address, which don't use relays.",
                log.direct_udp
            ),
        ));
    }
    if log.fetch_failures.len() > 1 {
        found.push(finding(
            Level::Note,
            format!(
                "The game failed to fetch the relay config {} times.",
                log.fetch_failures.len()
            ),
        ));
    }
    if log.condebug || log.lognetapi {
        found.push(finding(
            Level::Note,
            "Logging options are on. Remove them from Launch Options when you're finished, \
             because the log keeps growing while you play."
                .to_string(),
        ));
    }
    if let Some(age) = context.age_secs
        && age >= STALE_SECS
    {
        found.push(finding(
            Level::Note,
            format!(
                "This log was last written {}. These findings compare it with your current \
                 pick, so they only apply if that pick was on at the time.",
                ago(age)
            ),
        ));
    }
    found.sort_by_key(|item| item.level);
    found
}

fn ago(secs: u64) -> String {
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// Splits text into lines of at most `width`, for the fixed-width report.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

const WIDTH: usize = 78;

fn field(lines: &mut Vec<String>, name: &str, value: &str) {
    const NAME: usize = 11;
    for (index, part) in wrap(value, WIDTH - NAME - 2).into_iter().enumerate() {
        let label = if index == 0 { name } else { "" };
        lines.push(format!("  {label:<NAME$}{part}"));
    }
}

fn heading(lines: &mut Vec<String>, title: &str) {
    lines.push(String::new());
    lines.push(title.to_string());
}

fn file_section(log: &Log, context: &Context, lines: &mut Vec<String>) {
    heading(lines, "FILE");
    field(lines, "Path", &context.path);
    let written = context
        .age_secs
        .map_or(String::new(), |age| format!(", last written {}", ago(age)));
    let part = if context.truncated {
        ", only the end was read"
    } else {
        ""
    };
    field(
        lines,
        "Size",
        &format!("{} KB{written}{part}", context.bytes / 1000),
    );
    if let (Some(first), Some(last)) = (&log.first, &log.last) {
        let length = span(first, last).map_or(String::new(), |span| format!(" ({span})"));
        field(lines, "Time span", &format!("{first} to {last}{length}"));
    }
    if log.saw_options {
        let state = |on: bool| if on { "on" } else { "off" };
        field(
            lines,
            "Logging",
            &format!(
                "-condebug {}, -lognetapi {}",
                state(log.condebug),
                state(log.lognetapi)
            ),
        );
    }
}

fn findings_section(log: &Log, context: &Context, lines: &mut Vec<String>) {
    heading(lines, "FINDINGS");
    let found = findings(log, context);
    if found.is_empty() {
        lines.push("  Nothing to report.".to_string());
    }
    for item in found {
        for (index, part) in wrap(&item.text, WIDTH - 9).into_iter().enumerate() {
            let label = if index == 0 { item.level.label() } else { "" };
            lines.push(format!("  {label:<7}{part}"));
        }
    }
}

fn config_section(log: &Log, lines: &mut Vec<String>) {
    heading(lines, "RELAY CONFIG");
    let revision = log.revision.as_deref().unwrap_or("not loaded in this log");
    field(lines, "Revision", revision);
    if let Some(network) = &log.network {
        field(lines, "Last state", network);
    }
    for (at, reason) in &log.fetch_failures {
        let reason = if reason.is_empty() {
            "no reason given"
        } else {
            reason
        };
        field(
            lines,
            "Fetch",
            &format!("failed at {}: {reason}", clock(at)),
        );
    }
    for (at, relay, secs) in &log.offline {
        field(
            lines,
            "Offline",
            &format!("{relay} going offline in {secs} s, at {}", clock(at)),
        );
    }
}

fn measure_section(log: &Log, lines: &mut Vec<String>) {
    heading(lines, "PING MEASUREMENTS");
    if log.measures.is_empty() {
        lines.push("  None in this log.".to_string());
    }
    for measure in &log.measures {
        lines.push(format!(
            "  {}  {:>5} s  {} valid, {} great, {} good+, {} ok+, {} ignored",
            clock(&measure.at),
            measure.secs,
            measure.valid,
            measure.great,
            measure.good,
            measure.ok,
            measure.ignored
        ));
    }
}

fn ms(value: Option<u32>) -> String {
    value.map_or_else(|| "-".to_string(), |value| format!("{value} ms"))
}

fn table_section(log: &Log, context: &Context, lines: &mut Vec<String>) {
    let at = log
        .table_at
        .as_deref()
        .map_or(String::new(), |at| format!(" AT {}", clock(at)));
    heading(lines, &format!("PING TABLE{at}"));
    if log.table.is_empty() {
        lines.push("  None in this log.".to_string());
        return;
    }
    lines.push(format!(
        "  {:<8}{:<16}{:>9}{:>11}{:>9}",
        "Site", "Region", "Direct", "Via relay", "Penalty"
    ));
    let mut rows: Vec<&PingEntry> = log.table.iter().collect();
    rows.sort_by_key(|entry| entry.best().unwrap_or(u32::MAX));
    let open = open_sites(context);
    for entry in rows {
        let mark = if open.contains(&entry.site.as_str()) {
            " *"
        } else {
            ""
        };
        lines.push(format!(
            "  {:<8}{:<16}{:>9}{:>11}{:>9}",
            format!("{}{mark}", entry.site),
            regions::of(&entry.site, ""),
            ms(entry.direct),
            ms(entry.via),
            entry.penalty,
        ));
    }
    if !open.is_empty() {
        lines.push("  * open in dl-rs now".to_string());
    }
}

fn summary_fields(summary: &Summary, lines: &mut Vec<String>) {
    if let (Some(sent), Some(received)) = (&summary.sent, &summary.received) {
        field(
            lines,
            "Packets",
            &format!("{sent} sent, {received} received"),
        );
    }
    let mut lost = Vec::new();
    if let Some(dropped) = &summary.dropped {
        lost.push(format!("{} ({}) dropped here", dropped.0, dropped.1));
    }
    if let Some(dropped) = &summary.remote_dropped {
        lost.push(format!("{} ({}) dropped remotely", dropped.0, dropped.1));
    }
    if let Some(order) = &summary.out_of_order {
        lost.push(format!("{} ({}) out of order", order.0, order.1));
    }
    if !lost.is_empty() {
        field(lines, "Loss", &lost.join(", "));
    }
    if let [low, typical, _, high, worst] = summary.ping[..] {
        field(
            lines,
            "Ping",
            &format!(
                "{typical} ms typical. 5th {low}, 95th {high}, 98th {worst} ms (game's own figures)"
            ),
        );
    }
    if let [typical, _, low, worst] = summary.quality[..] {
        field(
            lines,
            "Quality",
            &format!("{typical}% typical. 5th {low}%, 2nd {worst}%"),
        );
    }
    if let Some(steady) = &summary.steady {
        field(lines, "Jitter", &format!("{steady} of readings under 1 ms"));
    }
}

fn connection_section(log: &Log, lines: &mut Vec<String>) {
    heading(lines, "CONNECTIONS THROUGH RELAYS");
    if log.connections.is_empty() {
        lines.push("  None in this log.".to_string());
    }
    for (index, conn) in log.connections.iter().enumerate() {
        let start = conn.start.as_deref().map_or("?", clock);
        let times = conn.end.as_deref().map_or_else(
            || format!("{start}, still open"),
            |end| format!("{start} to {}", clock(end)),
        );
        let length = conn
            .start
            .as_deref()
            .zip(conn.end.as_deref())
            .and_then(|(start, end)| span(start, end))
            .map_or(String::new(), |span| format!(" ({span})"));
        if index > 0 {
            lines.push(String::new());
        }
        lines.push(format!("  {}. {times}{length}", index + 1));
        if let Some(id) = &conn.match_id {
            field(lines, "Match", id);
        }
        for selected in &conn.selected {
            let role = selected
                .backup
                .map_or("Primary".to_string(), |number| format!("Backup {number}"));
            field(
                lines,
                &role,
                &format!(
                    "{} {} at {}. {} ms = {} (front+interior+remote). Rank {}, score {}",
                    selected.relay,
                    selected.address,
                    clock(&selected.at),
                    selected.ping,
                    selected.breakdown,
                    selected.rank,
                    selected.score
                ),
            );
        }
        for lost in &conn.lost {
            field(
                lines,
                "Lost",
                &format!(
                    "{} {} at {}. {}",
                    lost.role,
                    lost.relay,
                    clock(&lost.at),
                    lost.reason
                ),
            );
        }
        for (at, text) in &conn.events {
            field(lines, "Event", &format!("{} {text}", clock(at)));
        }
        if let Some(reason) = &conn.end_reason {
            field(lines, "Ended", reason);
        }
        summary_fields(&conn.summary, lines);
    }
    if log.local > 0 {
        lines.push(String::new());
        lines.push(format!(
            "  Not counted: {} local connection(s), such as the hideout.",
            log.local
        ));
    }
}

/// The whole report, as lines.
pub fn report(log: &Log, context: &Context) -> Vec<String> {
    let mut lines = vec!["dl-rs log report".to_string()];
    file_section(log, context, &mut lines);
    findings_section(log, context, &mut lines);
    config_section(log, &mut lines);
    measure_section(log, &mut lines);
    table_section(log, context, &mut lines);
    connection_section(log, &mut lines);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real lines from a Deadlock session under a strict Singapore pick, with IDs replaced.
    const SESSION: &str = "\u{feff}11/05 18:46:09 [RenderSystem] Determined driver version
11/05 18:46:13 [CommandLine] command line arguments:
11/05 18:46:13 [CommandLine] -steam -console -novid +password hunter2 -lognetapi -condebug
11/05 18:46:18 [SteamNetSockets] SDR network config fetch attempt #1 failed.  HTTP 504.  .  Trying again.
11/05 18:46:18 [SteamNetSockets] Got SDR network config.  Loaded revision 1793736599 OK
11/05 18:46:18 [SteamNetSockets] SDR RelayNetworkStatus:  avail=Attempting  config=OK  anyrelay=Attempting   (Performing ping measurement)
11/05 18:46:24 [SteamNetSockets] [#774331346 UDP steamid:76561190000000001@127.0.0.1:27015] connected
11/05 18:46:31 [SteamNetSockets] Ping measurement completed in 12.1s.  Relays: 1 valid, 0 great, 0 good+, 1 ok+, 0 ignored
11/05 18:46:31 [SteamNetSockets] SDR RelayNetworkStatus:  avail=OK  config=OK  anyrelay=OK   (OK.  Relays: 1 valid, 0 great, 0 good+, 1 ok+, 0 ignored)
11/05 18:46:31 [SteamNetSockets] Ping location: sgp=120+12/121+12,maa2=/152+12,hkg=/153+12,ams=/317+12
11/05 18:46:40 [SteamNetSockets] Relay lim#42 (1.2.3.4:27015) is going offline in 30 seconds
11/05 18:46:42 [SteamNetSockets] Received Steam datagram ticket for server steamid:76561190000000002 vport -1. match_id=131771756
11/05 18:46:42 Lobby 174210973408958091 for Match 131771756 created
11/05 18:46:43 [SteamNetSockets] [#774331346 UDP steamid:76561190000000001@127.0.0.1:27015 'server'] closed by app, linger requested but not needed (2055) NETWORK_DISCONNECT_LOOPDEACTIVATE
11/05 18:46:43 [SteamNetSockets] [#154429941 SDR server steamid:76561190000000002 vport 0] Requesting session from sgp#276 (103.10.124.121:27060).  Ping = 121 = 120+1 (front+back).  Rank=1 Score=156
11/05 18:46:43 [SteamNetSockets] [#154429941 SDR server steamid:76561190000000002 vport 0] Selecting sgp#276 (103.10.124.121:27060) as primary.  (Ping = 121 = 120+1+0 (front+interior+remote).  Rank=1 Score=133)
11/05 18:46:43 [SteamNetSockets] [#154429941 SDR server steamid:76561190000000002 vport 0] connected
11/05 18:54:02 [Networking] Summary of connection [#154429941 SDR server steamid:76561190000000002 vport 0 'server']:
11/05 18:54:02 [Networking]     End-to-end connection: connected
11/05 18:54:02 [Networking]         Remote host is in data center 'sgp'
11/05 18:54:02 [Networking]         Current rates:
11/05 18:54:02 [Networking]             Sent:  44.4 pkts/sec   9.3 K/sec
11/05 18:54:02 [Networking]             Recv:  66.4 pkts/sec  25.0 K/sec
11/05 18:54:02 [Networking]         Lifetime stats:
11/05 18:54:02 [Networking]             Totals
11/05 18:54:02 [Networking]                 Sent:     24,762 pkts       4,451,992 bytes
11/05 18:54:02 [Networking]                 Recv:     27,823 pkts      13,918,252 bytes
11/05 18:54:02 [Networking]                 Recv w seq:     27,822 pkts
11/05 18:54:02 [Networking]                 Dropped   :          0 pkts   0.00%
11/05 18:54:02 [Networking]                 OutOfOrder:          0 pkts   0.00%
11/05 18:54:02 [Networking]             Ping distribution:
11/05 18:54:02 [Networking]                 5th   50th   75th   95th   98th
11/05 18:54:02 [Networking]               120ms  121ms  121ms  121ms  121ms
11/05 18:54:02 [Networking]             Connection quality distribution:
11/05 18:54:02 [Networking]               50th  25th   5th   2nd
11/05 18:54:02 [Networking]               100%  100%  100%  100%
11/05 18:54:02 [Networking]             Latency variance histogram: (27793 total measurements)
11/05 18:54:02 [Networking]                       <1     1-2     2-5    5-10   10-20     >20
11/05 18:54:02 [Networking]                    27369     144      49      21     202       8
11/05 18:54:02 [Networking]                    98.5%    0.5%    0.2%    0.1%    0.7%    0.0%
11/05 18:54:02 [Networking]         Rate stats received from remote host 16.0s ago:
11/05 18:54:02 [Networking]             Sent:  63.9 pkts/sec  15.9 K/sec
11/05 18:54:02 [Networking]         Lifetime stats received from remote host 78.5s ago:
11/05 18:54:02 [Networking]                 Sent:     22,771 pkts      11,622,400 bytes
11/05 18:54:02 [Networking]                 Dropped   :          4 pkts   0.02%
11/05 18:54:02 [Networking]             Ping distribution:
11/05 18:54:02 [Networking]                 5th   50th   75th   95th   98th
11/05 18:54:02 [Networking]               999ms  999ms  999ms  999ms  999ms
11/05 18:53:54 [SteamNetSockets] [#154429941 SDR server steamid:76561190000000002 vport 0 'server'] Selecting tyo#172 (45.121.184.25:27048) as backup #1 (Ping = 274 = 190+84+0 (front+interior+remote).  Rank=2 Score=293)
11/05 18:54:02 [SteamNetSockets] [#154429941 SDR server steamid:76561190000000002 vport 0 'server'] closed by app, linger requested but not needed (1002) NETWORK_DISCONNECT_DISCONNECT_BY_USER
11/05 18:54:13 last line
";

    const LOST_BACKUP: &str = "11/05 18:30:00 [SteamNetSockets] [#1819906075 SDR server steamid:76561190000000002 vport 0] Selecting maa2#102 (155.133.225.19:27052) as backup #1 (Ping = 186 = 153+33+0 (front+interior+remote).  Rank=2 Score=201)
11/05 18:30:27 [SteamNetSockets] [#1819906075 SDR server steamid:76561190000000002 vport 0 'server'] Lost backup #1 maa2#102 (155.133.225.19:27052).  consecutive_ping_timeout
";

    fn context(pick: &str) -> Context {
        Context {
            path: "console.log".to_string(),
            bytes: 581_000,
            pick: pick.to_string(),
            ..Context::default()
        }
    }

    fn levels_with(found: &[Finding], text: &str) -> Vec<Level> {
        found
            .iter()
            .filter(|item| item.text.contains(text))
            .map(|item| item.level)
            .collect()
    }

    #[test]
    fn parses_the_session_overview() {
        let log = parse(SESSION);
        assert_eq!(log.first.as_deref(), Some("11/05 18:46:09"));
        assert_eq!(log.last.as_deref(), Some("11/05 18:54:13"));
        assert!(log.condebug && log.lognetapi);
        assert_eq!(log.revision.as_deref(), Some("1793736599"));
        assert_eq!(log.fetch_failures[0].1, "HTTP 504");
        assert_eq!(
            log.network.as_deref(),
            Some("avail=OK config=OK anyrelay=OK")
        );
        assert_eq!(log.offline[0].1, "lim#42");
        assert_eq!(log.local, 1);
    }

    #[test]
    fn parses_ping_measurements_and_the_table() {
        let log = parse(SESSION);
        let measure = &log.measures[0];
        assert_eq!(measure.secs, "12.1");
        assert_eq!((measure.valid, measure.ok, measure.ignored), (1, 1, 0));
        assert_eq!(log.table.len(), 4);
        assert_eq!(log.table[0].site, "sgp");
        assert_eq!(log.table[0].direct, Some(120));
        assert_eq!(log.table[0].via, Some(121));
        assert_eq!(log.table[0].penalty, 12);
        assert_eq!(log.table[1].direct, None);
        assert_eq!(log.table[1].via, Some(152));
        assert_eq!(log.table[1].penalty, 12);
    }

    #[test]
    fn parses_a_relay_connection_from_start_to_end() {
        let log = parse(SESSION);
        assert_eq!(log.connections.len(), 1, "only SDR connections count");
        let conn = &log.connections[0];
        assert_eq!(conn.match_id.as_deref(), Some("131771756"));
        assert!(conn.connected);
        let primary = conn.primary().unwrap();
        assert_eq!(primary.relay, "sgp#276");
        assert_eq!(primary.address, "103.10.124.121:27060");
        assert_eq!(primary.ping, 121);
        assert_eq!(primary.breakdown, "120+1+0");
        assert_eq!(primary.rank, "1");
        assert_eq!(primary.score, "133");
        let backup = conn.backups().next().unwrap();
        assert_eq!(backup.relay, "tyo#172");
        assert_eq!(backup.backup, Some(1));
        assert_eq!(conn.end_reason.as_deref(), Some("DISCONNECT_BY_USER"));
    }

    #[test]
    fn parses_the_closing_statistics_from_this_pc_not_the_remote() {
        let summary = parse(SESSION).connections[0].summary.clone();
        assert_eq!(summary.data_center.as_deref(), Some("sgp"));
        assert_eq!(summary.sent.as_deref(), Some("24,762"));
        assert_eq!(summary.received.as_deref(), Some("27,823"));
        assert_eq!(
            summary.dropped,
            Some(("0".to_string(), "0.00%".to_string()))
        );
        assert_eq!(
            summary.remote_dropped,
            Some(("4".to_string(), "0.02%".to_string()))
        );
        assert_eq!(summary.ping, [120, 121, 121, 121, 121]);
        assert_eq!(summary.quality, [100, 100, 100, 100]);
        assert_eq!(summary.steady.as_deref(), Some("98.5%"));
    }

    #[test]
    fn nothing_private_reaches_the_report() {
        let mut log = SESSION.to_string();
        log.push_str(
            "11/05 18:55:00 [SteamNetSockets] [#5 UDP steamid:76561190000000001@127.0.0.1:1 'A Player'] connected\n",
        );
        let text = report(&parse(&log), &context("sgp")).join("\n");
        assert!(!text.contains("7656119"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("A Player"), "{text}");
        assert!(!text.contains("steamid"), "{text}");
    }

    #[test]
    fn a_good_strict_session_passes() {
        let mut only_primary: Vec<&str> = SESSION.lines().collect();
        only_primary.retain(|line| !line.contains("backup #1"));
        let found = findings(&parse(&only_primary.join("\n")), &context("sgp"));
        assert_eq!(levels_with(&found, "relay(s) valid"), [Level::Ok]);
        assert_eq!(levels_with(&found, "is on your pick"), [Level::Ok]);
        assert_eq!(
            levels_with(&found, "expected with a strict pick"),
            [Level::Note]
        );
        assert_eq!(levels_with(&found, "Packet loss was low"), [Level::Ok]);
        assert!(
            found.iter().all(|item| item.level != Level::Check),
            "{found:?}"
        );
    }

    #[test]
    fn a_backup_outside_the_open_sites_is_flagged() {
        let found = findings(&parse(SESSION), &context("sgp"));
        assert_eq!(
            levels_with(&found, "outside your open sites"),
            [Level::Check]
        );
        assert_eq!(found[0].level, Level::Check, "worst comes first");
    }

    #[test]
    fn a_backup_that_is_open_is_fine() {
        let context = Context {
            backup: Some("tyo".to_string()),
            ..context("sgp")
        };
        let found = findings(&parse(SESSION), &context);
        assert!(levels_with(&found, "outside your open sites").is_empty());
        assert!(levels_with(&found, "expected with a strict pick").is_empty());
    }

    #[test]
    fn a_primary_elsewhere_is_flagged() {
        let found = findings(&parse(SESSION), &context("fra"));
        assert_eq!(levels_with(&found, "is not on your pick"), [Level::Check]);
    }

    #[test]
    fn too_many_valid_relays_means_the_block_was_off() {
        let log = "11/05 18:46:31 [SteamNetSockets] Ping measurement completed in 3.0s.  Relays: 33 valid, 20 great, 25 good+, 30 ok+, 0 ignored\n";
        let found = findings(&parse(log), &context("sgp"));
        assert_eq!(levels_with(&found, "33 relays were valid"), [Level::Check]);
    }

    #[test]
    fn no_valid_relays_is_flagged() {
        let log = "11/05 18:46:31 [SteamNetSockets] Ping measurement completed in 3.0s.  Relays: 0 valid, 0 great, 0 good+, 0 ok+, 0 ignored\n";
        let found = findings(&parse(log), &context("sgp"));
        assert_eq!(levels_with(&found, "No relay was valid"), [Level::Check]);
    }

    #[test]
    fn a_log_with_no_relay_connection_says_it_tests_nothing() {
        let log = "11/05 18:46:24 [SteamNetSockets] [#1 pipe] connected\n";
        let parsed = parse(log);
        assert_eq!(parsed.local, 1);
        let found = findings(&parsed, &context("sgp"));
        assert_eq!(levels_with(&found, "No SDR connection"), [Level::Note]);
    }

    #[test]
    fn no_pick_is_stated_rather_than_judged() {
        let found = findings(&parse(SESSION), &context(""));
        assert_eq!(levels_with(&found, "No pick is active"), [Level::Note]);
        assert!(levels_with(&found, "your pick").is_empty());
    }

    #[test]
    fn lost_backups_are_reported_with_their_reason() {
        let log = parse(LOST_BACKUP);
        let lost = &log.connections[0].lost[0];
        assert_eq!(lost.role, "backup #1");
        assert_eq!(lost.relay, "maa2#102");
        assert_eq!(lost.reason, "consecutive_ping_timeout");
        let found = findings(&log, &context("maa2"));
        assert_eq!(
            levels_with(&found, "consecutive_ping_timeout"),
            [Level::Note]
        );
    }

    #[test]
    fn route_comparison_only_applies_with_exitlag_on_the_pick() {
        let boosted = Context {
            exitlag: true,
            direct_ms: Some(356),
            ..context("sgp")
        };
        let found = findings(&parse(SESSION), &boosted);
        assert_eq!(levels_with(&found, "direct ping reads 356"), [Level::Note]);

        let slower = Context {
            direct_ms: Some(50),
            ..boosted.clone()
        };
        let found = findings(&parse(SESSION), &slower);
        assert_eq!(levels_with(&found, "slower than direct"), [Level::Check]);

        let plain = Context {
            exitlag: false,
            ..slower
        };
        let found = findings(&parse(SESSION), &plain);
        assert!(levels_with(&found, "direct ping reads").is_empty());
    }

    #[test]
    fn bad_connection_stats_are_flagged() {
        let log = "11/05 18:46:43 [SteamNetSockets] [#9 SDR server vport 0] connected
11/05 18:54:02 [Networking] Summary of connection [#9 SDR server vport 0 'server']:
11/05 18:54:02 [Networking]                 Dropped   :         500 pkts   2.50%
11/05 18:54:02 [Networking]             Ping distribution:
11/05 18:54:02 [Networking]                 5th   50th   75th   95th   98th
11/05 18:54:02 [Networking]               120ms  121ms  125ms  160ms  190ms
";
        let found = findings(&parse(log), &context("sgp"));
        assert_eq!(levels_with(&found, "dropped 2.50%"), [Level::Check]);
        assert_eq!(levels_with(&found, "Ping spiked"), [Level::Check]);
    }

    #[test]
    fn old_logs_are_called_out() {
        let old = Context {
            age_secs: Some(7200),
            ..context("sgp")
        };
        let found = findings(&parse(SESSION), &old);
        assert_eq!(levels_with(&found, "2 h ago"), [Level::Note]);
    }

    #[test]
    fn logging_left_on_is_mentioned() {
        let found = findings(&parse(SESSION), &context("sgp"));
        assert_eq!(levels_with(&found, "Logging options are on"), [Level::Note]);
    }

    #[test]
    fn garbage_is_ignored() {
        let log = parse(
            "not a log\n\n\u{0}\u{1}\n99/99 nonsense\n11/05 18:46:43 [SteamNetSockets] [#\n11/05 18:46:43 [SteamNetSockets] Ping location:\n",
        );
        assert!(log.connections.is_empty());
        assert!(log.table.is_empty());
        let text = report(&log, &context("sgp"));
        assert!(text[0].contains("report"));
    }

    #[test]
    fn the_report_is_tidy() {
        let text = report(&parse(SESSION), &context("sgp"));
        for heading in [
            "FILE",
            "FINDINGS",
            "RELAY CONFIG",
            "PING MEASUREMENTS",
            "PING TABLE AT 18:46:31",
            "CONNECTIONS THROUGH RELAYS",
        ] {
            assert!(text.iter().any(|line| line == heading), "{heading}");
        }
        for line in &text {
            assert!(line.is_ascii(), "{line}");
            assert!(line.len() <= 80, "too long: {line}");
        }
        let joined = text.join("\n");
        assert!(joined.contains("(7m 19s)"), "{joined}");
        assert!(joined.contains("sgp *"), "{joined}");
        assert!(joined.contains("Not counted: 1 local"), "{joined}");
    }

    #[test]
    fn the_route_follows_a_session() {
        let mut route = LiveRoute::default();
        assert_eq!(route.current("sgp"), None);
        let lines: Vec<&str> = SESSION.lines().collect();
        let until = |route: &mut LiveRoute, text: &str, from: &mut usize| {
            while *from < lines.len() {
                let line = lines[*from];
                route.feed(line);
                *from += 1;
                if line.contains(text) {
                    return;
                }
            }
            panic!("no line with {text}");
        };
        let mut at = 0;

        until(&mut route, "Ping location", &mut at);
        let measured = route.current("sgp").unwrap();
        assert_eq!(measured.ms, 120);
        assert_eq!(measured.at, "18:46");
        assert_eq!(measured.state, RouteState::Measured);
        assert_eq!(route.current("hkg").unwrap().ms, 153, "falls back to via");
        assert_eq!(route.current("fra"), None, "not in the table");

        until(&mut route, "as primary", &mut at);
        let live = route.current("sgp").unwrap();
        assert_eq!(live.ms, 121);
        assert_eq!(live.state, RouteState::InMatch);
        assert_eq!(route.current("fra").unwrap().site, "sgp", "names its site");

        until(
            &mut route,
            "closed by app, linger requested but not needed (1002)",
            &mut at,
        );
        let last = route.current("sgp").unwrap();
        assert_eq!(last.ms, 121);
        assert_eq!(last.at, "18:54");
        assert_eq!(last.state, RouteState::LastMatch);
    }

    #[test]
    fn the_route_ignores_local_connections() {
        let mut route = LiveRoute::default();
        route.feed("11/05 18:46:24 [SteamNetSockets] [#1 UDP steamid:1@127.0.0.1:27015] connected");
        route.feed("11/05 18:46:24 [SteamNetSockets] [#1 pipe 'x'] closed by peer (1): NETWORK_X");
        assert_eq!(route.current("sgp"), None);
    }

    #[test]
    fn a_newer_measurement_replaces_the_last_match() {
        let mut route = LiveRoute::default();
        route.feed("11/05 18:46:43 [SteamNetSockets] [#1 SDR server vport 0] Selecting sgp#1 (1.1.1.1:1) as primary.  (Ping = 121 = 120+1+0 (front+interior+remote).  Rank=1 Score=1)");
        route.feed(
            "11/05 18:50:00 [SteamNetSockets] [#1 SDR server vport 0] closed by app (1) NETWORK_X",
        );
        route.feed("11/05 19:10:00 [SteamNetSockets] Ping location: sgp=99+12/100+12");
        let now = route.current("sgp").unwrap();
        assert_eq!((now.ms, now.state), (99, RouteState::Measured));
    }

    #[test]
    fn reads_library_folders_from_steam() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"1"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            libraries(vdf),
            [r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"]
        );
    }
}
