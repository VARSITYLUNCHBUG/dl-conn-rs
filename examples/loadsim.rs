//! Game-shaped UDP traffic for testing dl-conn-rs under load. Not part of the app.
//!
//! Like a game client, it uses one socket: a steady tick stream to the relay it's playing through,
//! plus small once-a-second pings to other relays, the way Steam's relay selection probes them.
//! This reaches real servers, so rates are capped at what a heavy game client sends. Sustained
//! high-rate load uses `--echo` instead, which stays on this PC.
//!
//!   loadsim --open <ip> [--probe <ip,ip,...>] [--secs 30] [--pps 256] [--size 1200] [--warmup 0]
//!   loadsim --echo <secs>

use std::net::{Ipv4Addr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

const RELAY_PORT: u16 = 27015;
const MAX_PPS: u32 = 512;
const MAX_SIZE: usize = 1200;
const MAX_SECS: u64 = 120;
const PROBE_SIZE: usize = 64;
const PROBE_EVERY: Duration = Duration::from_secs(1);
/// Below this, spin rather than sleep, because Windows sleeps in steps of about 15 ms.
const SPIN_UNDER: Duration = Duration::from_millis(2);

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .map(String::as_str)
    };
    let number = |name: &str, default: u64| -> Result<u64, String> {
        value(name).map_or(Ok(default), |text| {
            text.parse().map_err(|_| format!("bad {name}: {text}"))
        })
    };
    if let Some(secs) = value("--echo") {
        let secs = secs.parse().map_err(|_| format!("bad --echo: {secs}"))?;
        return echo(Duration::from_secs(secs)).map_err(|error| error.to_string());
    }
    let open: Ipv4Addr = value("--open")
        .ok_or("--open <ip> is required")?
        .parse()
        .map_err(|_| "bad --open")?;
    let probes: Vec<Ipv4Addr> = value("--probe")
        .unwrap_or_default()
        .split(',')
        .filter(|text| !text.is_empty())
        .map(|text| text.parse().map_err(|_| format!("bad --probe: {text}")))
        .collect::<Result<_, _>>()?;
    let secs = number("--secs", 30)?.min(MAX_SECS);
    let pps = u32::try_from(number("--pps", 256)?)
        .unwrap_or(MAX_PPS)
        .min(MAX_PPS);
    let size = usize::try_from(number("--size", 1200)?)
        .unwrap_or(MAX_SIZE)
        .min(MAX_SIZE);
    thread::sleep(Duration::from_secs(number("--warmup", 0)?));
    stream(open, &probes, Duration::from_secs(secs), pps.max(1), size)
        .map_err(|error| error.to_string())
}

fn stream(
    open: Ipv4Addr,
    probes: &[Ipv4Addr],
    duration: Duration,
    pps: u32,
    size: usize,
) -> std::io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    let tick = vec![0u8; size];
    let ping = [0u8; PROBE_SIZE];
    let interval = Duration::from_secs(1) / pps;
    let (mut sent, mut probed, mut errors) = (0u64, 0u64, 0u64);
    let mut lateness = Vec::new();
    let start = Instant::now();
    let mut next_probe = start;
    let mut due = start;
    while start.elapsed() < duration {
        let wait = due.saturating_duration_since(Instant::now());
        if wait > SPIN_UNDER {
            thread::sleep(wait - SPIN_UNDER);
        }
        while Instant::now() < due {
            thread::yield_now();
        }
        lateness.push(due.elapsed());
        match socket.send_to(&tick, (open, RELAY_PORT)) {
            Ok(_) => sent += 1,
            Err(_) => errors += 1,
        }
        if Instant::now() >= next_probe {
            for probe in probes {
                match socket.send_to(&ping, (*probe, RELAY_PORT)) {
                    Ok(_) => probed += 1,
                    Err(_) => errors += 1,
                }
            }
            next_probe += PROBE_EVERY;
        }
        due += interval;
    }
    let (p50, p99, max) = percentiles(&mut lateness);
    println!(
        "sent={sent} probes={probed} errors={errors} pps={pps} size={size} late_p50_us={p50} late_p99_us={p99} late_max_us={max}"
    );
    Ok(())
}

/// Bounces packets off a local echo as fast as they'll go, measuring the round trip.
fn echo(duration: Duration) -> std::io::Result<()> {
    let server = UdpSocket::bind("127.0.0.1:0")?;
    let address = server.local_addr()?;
    thread::spawn(move || {
        let mut buffer = [0u8; MAX_SIZE];
        while let Ok((length, from)) = server.recv_from(&mut buffer) {
            let _ = server.send_to(&buffer[..length], from);
        }
    });
    let client = UdpSocket::bind("127.0.0.1:0")?;
    client.connect(address)?;
    client.set_read_timeout(Some(Duration::from_secs(1)))?;
    let packet = [0u8; MAX_SIZE];
    let mut reply = [0u8; MAX_SIZE];
    let mut round_trips = Vec::new();
    let mut lost = 0u64;
    let start = Instant::now();
    while start.elapsed() < duration {
        let sent_at = Instant::now();
        client.send(&packet)?;
        match client.recv(&mut reply) {
            Ok(_) => round_trips.push(sent_at.elapsed()),
            Err(_) => lost += 1,
        }
    }
    let pps = round_trips.len() as u64 / duration.as_secs().max(1);
    let (p50, p99, max) = percentiles(&mut round_trips);
    println!("echo_pps={pps} lost={lost} rtt_p50_us={p50} rtt_p99_us={p99} rtt_max_us={max}");
    Ok(())
}

fn percentiles(samples: &mut [Duration]) -> (u128, u128, u128) {
    if samples.is_empty() {
        return (0, 0, 0);
    }
    samples.sort_unstable();
    let at = |fraction: f64| samples[((samples.len() - 1) as f64 * fraction) as usize].as_micros();
    (at(0.5), at(0.99), at(1.0))
}
