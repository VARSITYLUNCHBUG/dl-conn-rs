//! Snapshots behind the diagnostic commands: running processes, routes and relay latency.

use crate::from_wide;
use crate::sdr::{Pops, blocklist};
use crate::wfp::DROPS;
use std::collections::BTreeMap;
use std::mem::{size_of, zeroed};
use std::net::Ipv4Addr;
use std::ptr::null;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::NetworkManagement::IpHelper::*;
use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_INET};
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;

const WATCHED: [&str; 8] = [
    "deadlock.exe",
    "project8.exe",
    "cs2.exe",
    "dota2.exe",
    "hl2.exe",
    "tf_win64.exe",
    "steam.exe",
    "exitlag.exe",
];
const PING_TIMEOUT_MS: u32 = 800;
const IP_REQ_TIMED_OUT: u32 = 11010;
/// Same size as ping.exe's default payload.
const PROBE: &[u8; 32] = b"dl-rs relay latency probe 32 byt";

/// A filter on this PC refused the packet. On a picked relay that means another tool's rule.
pub const IP_GENERAL_FAILURE: u32 = 11050;

/// Everything at once: processes, the open sites and their relays, and traffic blocked since the
/// last one.
pub fn report(pops: &Pops, open: &[&str]) -> Vec<String> {
    let mut lines = vec![format!("-- processes: {}", processes())];
    let blocks = blocklist(pops, open);
    if blocks.is_empty() {
        lines.push("-- picking: nothing, all relays open".to_string());
    } else {
        let blocked: usize = blocks.values().map(Vec::len).sum();
        lines.push(format!(
            "-- picking: {}: {blocked} relays blocked across {} sites",
            open.join(" + "),
            blocks.len()
        ));
        for code in open {
            let pop = &pops[*code];
            lines.push(format!(
                "   {code} {}: {} relays open",
                pop.desc,
                pop.ips.len()
            ));
            let ips: Vec<Ipv4Addr> = pop.ips.iter().copied().collect();
            for (rtt, ip) in probe(&ips) {
                lines.push(format!("   {ip:<16} {:>15}  {}", latency(rtt), route(ip)));
            }
        }
    }
    lines.extend(drops(pops));
    lines
}

/// Latency per relay, best first.
pub fn pings(ips: &[Ipv4Addr]) -> Vec<String> {
    probe(ips)
        .into_iter()
        .map(|(rtt, ip)| format!("{ip:<16} {:>15}", latency(rtt)))
        .collect()
}

/// The route to each relay.
pub fn routes(ips: &[Ipv4Addr]) -> Vec<String> {
    ips.iter()
        .map(|ip| format!("{ip:<16} {}", route(*ip)))
        .collect()
}

/// Pings relays at once so a dead one can't stall the result, best first.
fn probe(ips: &[Ipv4Addr]) -> Vec<(Result<u32, u32>, Ipv4Addr)> {
    let mut rows: Vec<_> = std::thread::scope(|scope| {
        let probes: Vec<_> = ips
            .iter()
            .map(|ip| scope.spawn(move || (ping(*ip), *ip)))
            .collect();
        probes.into_iter().filter_map(|p| p.join().ok()).collect()
    });
    rows.sort_by_key(|(rtt, ip)| (rtt.is_err(), rtt.ok(), *ip));
    rows
}

fn latency(rtt: Result<u32, u32>) -> String {
    match rtt {
        Ok(ms) => format!("{ms} ms"),
        Err(IP_REQ_TIMED_OUT) => "timeout".to_string(),
        Err(IP_GENERAL_FAILURE) => "blocked locally".to_string(),
        Err(code) => format!("icmp error {code}"),
    }
}

fn drops(pops: &Pops) -> Vec<String> {
    let owners: BTreeMap<Ipv4Addr, &str> = pops
        .iter()
        .flat_map(|(code, pop)| pop.ips.iter().map(move |ip| (*ip, code.as_str())))
        .collect();
    let drained = std::mem::take(&mut *DROPS.lock().unwrap());
    // The subscription sees every drop on the system; only relay traffic matters here.
    drained
        .into_iter()
        .filter_map(|((ip, app), count)| {
            let pop = owners.get(&ip)?;
            Some(format!("   blocked {app} -> {pop} {ip} x{count}"))
        })
        .collect()
}

pub fn processes() -> String {
    let mut found = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return "unavailable".into();
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let name = from_wide(&entry.szExeFile);
            if WATCHED.contains(&name.to_lowercase().as_str()) {
                found.push(format!("{name}#{}", entry.th32ProcessID));
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    if found.is_empty() {
        return "none watched".into();
    }
    found.join("  ")
}

/// The OS's chosen path to a relay: interface alias and next hop.
fn route(ip: Ipv4Addr) -> String {
    unsafe {
        let mut destination: SOCKADDR_INET = zeroed();
        destination.Ipv4.sin_family = AF_INET;
        destination.Ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes(ip.octets());
        let (mut row, mut source): (MIB_IPFORWARD_ROW2, SOCKADDR_INET) = (zeroed(), zeroed());
        if GetBestRoute2(null(), 0, null(), &destination, 0, &mut row, &mut source) != 0 {
            return "no route".into();
        }
        let mut alias = [0u16; 257];
        ConvertInterfaceLuidToAlias(&row.InterfaceLuid, alias.as_mut_ptr(), alias.len());
        let hop = Ipv4Addr::from(row.NextHop.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes());
        format!("via {} gw {hop}", from_wide(&alias))
    }
}

/// One ICMP echo, in milliseconds. A game's real path can differ, especially through ExitLag.
pub fn ping(ip: Ipv4Addr) -> Result<u32, u32> {
    unsafe {
        let icmp = IcmpCreateFile();
        if icmp == INVALID_HANDLE_VALUE {
            return Err(GetLastError());
        }
        // Reply struct plus payload plus 8 bytes for an ICMP error, per the API docs.
        let mut reply = [0u64; 16];
        let sent = IcmpSendEcho(
            icmp,
            u32::from_ne_bytes(ip.octets()),
            PROBE.as_ptr().cast(),
            PROBE.len() as u16,
            null(),
            reply.as_mut_ptr().cast(),
            size_of::<[u64; 16]>() as u32,
            PING_TIMEOUT_MS,
        );
        let error = GetLastError();
        IcmpCloseHandle(icmp);
        let reply = &*reply.as_ptr().cast::<ICMP_ECHO_REPLY>();
        match if sent > 0 { reply.Status } else { error } {
            0 => Ok(reply.RoundTripTime),
            code => Err(code),
        }
    }
}
