//! Tests against the real WFP engine and real relays. They need admin rights and network access,
//! so they are ignored by default and run through `scripts/live-test.ps1`.

use crate::diag::{IP_GENERAL_FAILURE, ping};
use crate::sdr::{Blocks, Client};
use crate::wfp::{DROPS, Engine};
use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

const STAMP: &str = "live-test 0";
const RELAY_PORT: u16 = 27015;
const DNS_TARGET: Ipv4Addr = Ipv4Addr::new(208, 67, 222, 222);
const DNS_OTHER: Ipv4Addr = Ipv4Addr::new(9, 9, 9, 9);

/// Relays can lose a ping, so one miss doesn't count as unreachable.
fn reachable(ip: Ipv4Addr) -> bool {
    (0..3).any(|_| ping(ip).is_ok())
}

/// Relays in two PoPs that answer right now, so another tool's rules can't confuse the result.
fn two_live_relays() -> (Ipv4Addr, Ipv4Addr) {
    let pops = Client::new().unwrap().config(730).unwrap();
    let mut live = pops
        .values()
        .filter_map(|pop| pop.ips.iter().copied().find(|ip| reachable(*ip)));
    (live.next().unwrap(), live.next().unwrap())
}

fn open_clean() -> Engine {
    let engine = Engine::open().unwrap();
    assert!(
        engine.stamp().is_none(),
        "a real pick is active; not touching it"
    );
    engine
}

fn block(engine: &Engine, ip: Ipv4Addr) {
    let blocks = Blocks::from([("test", vec![ip])]);
    engine.apply(STAMP, &blocks).unwrap();
}

/// An unconnected send reports success even when dropped, so the result is ignored. Drop events
/// are what prove a block.
fn udp_send(ip: Ipv4Addr) {
    let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    let _ = socket.send_to(b"dl-rs test", (ip, RELAY_PORT));
}

/// A connected UDP socket to a public DNS resolver. Relays ignore stray packets, so they can't
/// show whether UDP got through; a resolver always answers, which makes a true round-trip check.
fn dns_socket(ip: Ipv4Addr) -> UdpSocket {
    let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(700)))
        .unwrap();
    socket.connect((ip, 53)).unwrap();
    socket
}

/// Sends an A query for example.com and reports whether any answer came back.
fn dns_answers(socket: &UdpSocket) -> bool {
    const QUERY: &[u8] =
        b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01";
    let mut reply = [0u8; 512];
    (0..3).any(|_| socket.send(QUERY).is_ok() && socket.recv(&mut reply).is_ok())
}

/// Sends UDP to `target` and `other` until a drop for `target` is reported.
fn saw_udp_drop(engine: &Engine, target: Ipv4Addr, other: Ipv4Addr) -> bool {
    DROPS.lock().unwrap().clear();
    engine.watch_drops(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seen = false;
    while !seen && Instant::now() < deadline {
        udp_send(target);
        udp_send(other);
        std::thread::sleep(Duration::from_millis(500));
        seen = DROPS.lock().unwrap().keys().any(|(ip, _)| *ip == target);
    }
    engine.watch_drops(false).unwrap();
    let drops = DROPS.lock().unwrap();
    println!("drops: {drops:?}");
    assert!(
        !drops.keys().any(|(ip, _)| *ip == other),
        "unblocked relay was dropped"
    );
    seen
}

struct ClearOnDrop<'a>(&'a Engine);

impl Drop for ClearOnDrop<'_> {
    fn drop(&mut self) {
        let _ = self.0.clear();
    }
}

#[test]
#[ignore = "needs admin"]
fn blocks_only_the_target_and_clears_cleanly() {
    let engine = open_clean();
    let _guard = ClearOnDrop(&engine);
    let (target, neighbour) = two_live_relays();
    println!("target {target}, neighbour {neighbour}");

    block(&engine, target);
    assert_eq!(engine.stamp().as_deref(), Some(STAMP));
    assert!(!engine.has_persistent_objects(), "would survive reboot");
    assert_eq!(ping(target), Err(IP_GENERAL_FAILURE));
    assert!(reachable(neighbour), "other relays must stay open");

    // Swapping to a different block must take effect in both directions.
    block(&engine, neighbour);
    assert!(reachable(target), "old block should go after a swap");
    assert_eq!(ping(neighbour), Err(IP_GENERAL_FAILURE));

    engine.clear().unwrap();
    assert!(engine.stamp().is_none());
    assert!(reachable(neighbour), "clear should restore access");
}

#[test]
#[ignore = "needs admin"]
fn reports_dropped_packets() {
    let engine = open_clean();
    let _guard = ClearOnDrop(&engine);
    let (target, other) = two_live_relays();
    block(&engine, target);
    assert!(saw_udp_drop(&engine, target, other), "no UDP drop event");
}

/// A flow already passing traffic must stop when the block lands and recover when it lifts, and
/// a second destination must never be affected.
#[test]
#[ignore = "needs admin"]
fn live_udp_flow_is_cut_and_restored() {
    let engine = open_clean();
    let _guard = ClearOnDrop(&engine);
    let (flow, other) = (dns_socket(DNS_TARGET), dns_socket(DNS_OTHER));
    assert!(dns_answers(&flow), "flow must work before the block");
    assert!(dns_answers(&other), "other must work before the block");

    block(&engine, DNS_TARGET);
    assert!(!dns_answers(&flow), "live flow should be cut");
    assert!(
        !dns_answers(&dns_socket(DNS_TARGET)),
        "new flow should be cut"
    );
    assert!(dns_answers(&other), "other destination must be unaffected");

    engine.clear().unwrap();
    assert!(dns_answers(&flow), "live flow should recover");
    assert!(dns_answers(&dns_socket(DNS_TARGET)), "new flow should work");
}

/// Leaves a block behind on purpose; `survives_exit_part2` checks it from a new process.
#[test]
#[ignore = "needs admin; run by the live-test script"]
fn survives_exit_part1() {
    let engine = open_clean();
    let (target, _) = two_live_relays();
    block(&engine, target);
    println!("left {target} blocked for part 2");
}

#[test]
#[ignore = "needs admin; run by the live-test script"]
fn survives_exit_part2() {
    let engine = Engine::open().unwrap();
    let _guard = ClearOnDrop(&engine);
    assert_eq!(
        engine.stamp().as_deref(),
        Some(STAMP),
        "block did not outlive its process"
    );
    assert!(!engine.has_persistent_objects(), "would survive reboot");
}

/// Safety net: the live-test script always finishes with this.
#[test]
#[ignore = "needs admin; run by the live-test script"]
fn clear_any_block() {
    Engine::open().unwrap().clear().unwrap();
}

/// Prints the report for the log on this PC. Read-only and needs no admin, so it's safe in a match.
#[test]
#[ignore = "reads the real game log; run by hand"]
fn prints_the_report_for_the_real_log() {
    let (text, context) = crate::diag::read_log(None).unwrap();
    let context = crate::analysis::Context {
        pick: std::env::var("DLRS_PICK").unwrap_or_default(),
        ..context
    };
    let log = crate::analysis::parse(&text);
    for line in crate::analysis::report(&log, &context) {
        println!("{line}");
    }
}
