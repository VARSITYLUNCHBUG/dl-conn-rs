//! Snapshots for the debug console: running processes, routes and relay latency.

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

pub fn report(pops: &Pops, pick: &str) {
    log!("-- processes: {}", processes());
    match pops.get(pick) {
        Some(pop) => {
            let blocks = blocklist(pops, pick);
            let blocked: usize = blocks.values().map(Vec::len).sum();
            log!(
                "-- picking: {pick} {}: {} relays open, {blocked} blocked across {} PoPs",
                pop.desc,
                pop.ips.len(),
                blocks.len()
            );
            relays(pop.ips.iter().copied().collect());
        }
        None => log!("-- picking: nothing, all relays open"),
    }
    drops(pops);
}

/// Pings the pick's relays at once so a dead relay can't stall the report, best first.
fn relays(ips: Vec<Ipv4Addr>) {
    let mut rows: Vec<_> = std::thread::scope(|scope| {
        let probes: Vec<_> = ips
            .into_iter()
            .map(|ip| scope.spawn(move || (ping(ip), ip, route(ip))))
            .collect();
        probes.into_iter().filter_map(|p| p.join().ok()).collect()
    });
    rows.sort_by_key(|(rtt, ip, _)| (rtt.is_err(), rtt.ok(), *ip));
    for (rtt, ip, route) in rows {
        let rtt = rtt.map_or_else(ping_error, |ms| format!("{ms} ms"));
        log!("   {ip:<16} {rtt:>15}  {route}");
    }
}

fn ping_error(code: u32) -> String {
    match code {
        IP_REQ_TIMED_OUT => "timeout".to_string(),
        IP_GENERAL_FAILURE => "blocked locally".to_string(),
        code => format!("icmp error {code}"),
    }
}

fn drops(pops: &Pops) {
    let owners: BTreeMap<Ipv4Addr, &str> = pops
        .iter()
        .flat_map(|(code, pop)| pop.ips.iter().map(move |ip| (*ip, code.as_str())))
        .collect();
    let drained = std::mem::take(&mut *DROPS.lock().unwrap());
    for ((ip, app), count) in drained {
        // The subscription sees every drop on the system; only relay traffic matters here.
        if let Some(pop) = owners.get(&ip) {
            log!("   blocked {app} -> {pop} {ip} x{count}");
        }
    }
}

fn processes() -> String {
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

/// One ICMP echo, in milliseconds. Sent directly, so it shows the ISP path, not ExitLag's.
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
