//! Background work: the fetch loop and the single owner of the WFP engine.

use crate::diag;
use crate::sdr::{self, Client, Pops};
use crate::ui::{self, View};
use crate::wfp::Engine;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Steam serves this API with `max-age` up to 300 s, so polling faster would see nothing new.
const REFRESH: Duration = Duration::from_secs(300);
/// Picking pulls the next fetch forward, but never closer than this to the last one.
const MIN_FETCH_GAP: Duration = Duration::from_secs(30);
const RETRY_MIN: Duration = Duration::from_secs(5);
const MAX_JITTER_MS: u32 = 15_000;
const CONSOLE_TICK: Duration = Duration::from_secs(5);

pub enum Cmd {
    Pick(String),
    Relays(Pops),
    Offline,
    Console(bool),
}

pub fn spawn(engine: Engine) -> Sender<Cmd> {
    let (tx, rx) = channel();
    let (wake_tx, wake_rx) = channel();
    let fetch_tx = tx.clone();
    thread::spawn(move || fetch_loop(&fetch_tx, &wake_rx));
    let pick = current_pick(&engine);
    thread::spawn(move || {
        Worker {
            engine,
            pops: Pops::new(),
            pick,
            console: false,
            offline: false,
            error: None,
            wake: wake_tx,
        }
        .run(&rx);
    });
    tx
}

/// Recovers the pick from WFP, so reopening the app mid-session picks up where it left off.
fn current_pick(engine: &Engine) -> String {
    let stamp = engine.stamp().unwrap_or_default();
    stamp.split(' ').next().unwrap_or_default().to_string()
}

/// Spreads refreshes out so many clients don't hit the API in lockstep.
fn jitter() -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |now| now.subsec_nanos());
    Duration::from_millis(u64::from(nanos % MAX_JITTER_MS))
}

/// Keeps each app's last good config, so one failed fetch never shrinks the merged relay set.
fn fetch_loop(tx: &Sender<Cmd>, wake: &Receiver<()>) {
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
                    log!("fetch app {app}: {} PoPs with relays", pops.len());
                    *slot = Some(pops);
                }
                Err(error) => {
                    log!("fetch app {app}: {error}");
                    all_ok = false;
                }
            }
        }
        let cmd = if cache.iter().any(Option::is_some) {
            Cmd::Relays(sdr::merge(cache.iter().flatten()))
        } else {
            Cmd::Offline
        };
        if tx.send(cmd).is_err() {
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
                Ok(()) => next = next.min(earliest),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

struct Worker {
    engine: Engine,
    pops: Pops,
    pick: String,
    console: bool,
    offline: bool,
    error: Option<String>,
    wake: Sender<()>,
}

impl Worker {
    fn run(mut self, rx: &Receiver<Cmd>) {
        log!("start: active pick {:?}", self.pick);
        self.publish();
        loop {
            let wait = if self.console {
                CONSOLE_TICK
            } else {
                Duration::MAX
            };
            match rx.recv_timeout(wait) {
                Ok(Cmd::Pick(pick)) => {
                    if pick != self.pick {
                        log!("pick: {pick:?}");
                        self.pick = pick;
                        self.reconcile();
                        self.publish();
                        let _ = self.wake.send(());
                    }
                }
                Ok(Cmd::Relays(pops)) => {
                    if pops != self.pops {
                        let relays: usize = pops.values().map(|pop| pop.ips.len()).sum();
                        log!("relays: {} PoPs, {relays} addresses", pops.len());
                    }
                    self.pops = pops;
                    self.offline = false;
                    // Runs even when nothing changed, which restores filters removed behind
                    // our back; an unchanged stamp means no write.
                    self.reconcile();
                    self.publish();
                }
                Ok(Cmd::Offline) => {
                    self.offline = true;
                    self.publish();
                }
                Ok(Cmd::Console(on)) => {
                    self.console = on;
                    if let Err(error) = self.engine.watch_drops(on) {
                        log!("drop watcher: {error}");
                    }
                    if on {
                        diag::report(&self.pops, &self.pick);
                    }
                }
                Err(RecvTimeoutError::Timeout) => diag::report(&self.pops, &self.pick),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Makes WFP match the pick. Filter writes make Windows re-check every live flow, so this
    /// only writes when the stamp differs. Data problems leave the current block in place.
    fn reconcile(&mut self) {
        self.error = None;
        let result = if self.pick.is_empty() {
            self.engine.clear()
        } else if !self.pops.contains_key(&self.pick) {
            if !self.pops.is_empty() {
                log!("pick {:?} not in config; keeping current block", self.pick);
            }
            Ok(())
        } else {
            let blocks = sdr::blocklist(&self.pops, &self.pick);
            let stamp = sdr::stamp(&self.pick, &blocks);
            if self.engine.stamp().as_deref() == Some(&stamp) {
                Ok(())
            } else {
                log!("apply: {stamp}, {} PoP filters", blocks.len());
                self.engine.apply(&stamp, &blocks)
            }
        };
        if let Err(error) = result {
            log!("apply failed: {error}");
            self.error = Some(error);
        }
    }

    fn publish(&self) {
        let mut items: Vec<(String, String)> = self
            .pops
            .iter()
            .map(|(code, pop)| (code.clone(), format!("{} - {code}", pop.desc)))
            .collect();
        // A pick the config no longer lists stays visible, so the dropdown never misstates it.
        if !self.pick.is_empty() && !self.pops.contains_key(&self.pick) {
            items.push((self.pick.clone(), self.pick.clone()));
        }
        items.sort_by_key(|(_, label)| (label.contains("(China)"), label.clone()));
        items.insert(0, (String::new(), "No block - all servers".to_string()));
        let selected = items
            .iter()
            .position(|(code, _)| *code == self.pick)
            .unwrap_or(0);
        ui::publish(View {
            items,
            selected,
            title: self.title(),
        });
    }

    fn title(&self) -> String {
        if let Some(error) = &self.error {
            return format!("dl-rs - {error}");
        }
        let offline = if self.offline { " (offline)" } else { "" };
        if self.pick.is_empty() {
            if self.pops.is_empty() && !self.offline {
                return "dl-rs - loading".to_string();
            }
            return format!("dl-rs - no block{offline}");
        }
        match self.pops.get(&self.pick) {
            Some(pop) => {
                let blocked: usize = sdr::blocklist(&self.pops, &self.pick)
                    .values()
                    .map(Vec::len)
                    .sum();
                format!("dl-rs - {} - {blocked} blocked{offline}", pop.desc)
            }
            None => format!("dl-rs - {} active{offline}", self.pick),
        }
    }
}
