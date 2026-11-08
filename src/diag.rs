//! Snapshots behind the diagnostic commands: running processes, routes and relay latency.

use crate::analysis::{self, Context};
use crate::from_wide;
use crate::sdr::{Pops, blocklist};
use crate::wfp::DROPS;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::mem::{size_of, zeroed};
use std::net::Ipv4Addr;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::NetworkManagement::IpHelper::*;
use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_INET};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_ProgramFilesX86, SHGetKnownFolderPath,
};
use windows_sys::core::{GUID, PCWSTR, PWSTR};
use windows_sys::w;

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
/// Games whose relay traffic dl-conn-rs affects. A running one means a match may be live.
const GAMES: [&str; 6] = [
    "deadlock.exe",
    "project8.exe",
    "cs2.exe",
    "dota2.exe",
    "hl2.exe",
    "tf_win64.exe",
];
/// Deadlock's Steam app id, whose manifest names its real install folder.
const DEADLOCK_APP: &str = "1422450";
const DEADLOCK_FOLDER: &str = "Deadlock";
const LOG_IN_GAME: &str = r"game\citadel\console.log";
/// Lets tests keep their settings and logs away from the real ones.
const DATA_DIR_OVERRIDE: &str = "DL_CONN_RS_DATA_DIR";
const APP_FOLDER: &str = "dl-conn-rs";
/// The end of the log holds the current session, and this keeps a forgotten log from stalling
/// commands while it's read.
const MAX_LOG_BYTES: u64 = 16 << 20;
const PING_TIMEOUT_MS: u32 = 800;
const IP_REQ_TIMED_OUT: u32 = 11010;
/// Same size as ping.exe's default payload.
const PROBE: &[u8; 32] = b"dl-conn-rs latency probe 32 byte";

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
pub fn probe(ips: &[Ipv4Addr]) -> Vec<(Result<u32, u32>, Ipv4Addr)> {
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
    let Some(running) = running() else {
        return "unavailable".into();
    };
    let found: Vec<String> = running
        .iter()
        .filter(|(name, _)| WATCHED.contains(&name.to_lowercase().as_str()))
        .map(|(name, id)| format!("{name}#{id}"))
        .collect();
    if found.is_empty() {
        return "none watched".into();
    }
    found.join("  ")
}

pub fn exitlag_running() -> bool {
    any_running(&["exitlag.exe"])
}

pub fn game_running() -> bool {
    any_running(&GAMES)
}

fn any_running(names: &[&str]) -> bool {
    running().is_some_and(|running| {
        running
            .iter()
            .any(|(name, _)| names.contains(&name.to_lowercase().as_str()))
    })
}

/// Every running process's name and id.
fn running() -> Option<Vec<(String, u32)>> {
    let mut found = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            found.push((from_wide(&entry.szExeFile), entry.th32ProcessID));
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    Some(found)
}

/// Opens a file the game may hold open, or rewrite, while it's being read.
pub fn open_shared(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

/// A folder Windows knows the real location of, wherever this PC keeps it.
fn known_folder(id: &GUID) -> Option<PathBuf> {
    let mut path: PWSTR = null_mut();
    unsafe {
        let status = SHGetKnownFolderPath(id, 0, null_mut(), &mut path);
        let found = (status == 0 && !path.is_null()).then(|| {
            let length = (0..).take_while(|at| *path.add(*at) != 0).count();
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(
                path, length,
            )))
        });
        CoTaskMemFree(path.cast());
        found
    }
}

/// Where this app keeps its settings and log. `None` if Windows doesn't say, and then settings
/// last only for this session.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(folder) = env::var_os(DATA_DIR_OVERRIDE) {
        return Some(PathBuf::from(folder));
    }
    known_folder(&FOLDERID_LocalAppData).map(|base| base.join(APP_FOLDER))
}

fn registry_text(root: HKEY, key: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut size = size_of::<[u16; 1024]>() as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            key,
            value,
            RRF_RT_REG_SZ,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    (status == 0).then(|| from_wide(&buffer).replace('/', "\\"))
}

/// Where Steam might be. HKCU is what Steam records for its user, but it belongs to someone else when
/// a standard user elevates with an administrator's password, so HKLM is checked as well.
fn steam_folders() -> Vec<PathBuf> {
    let recorded = [
        registry_text(
            HKEY_CURRENT_USER,
            w!("Software\\Valve\\Steam"),
            w!("SteamPath"),
        ),
        registry_text(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\WOW6432Node\\Valve\\Steam"),
            w!("InstallPath"),
        ),
    ];
    let default = known_folder(&FOLDERID_ProgramFilesX86).map(|folder| folder.join("Steam"));
    let mut folders: Vec<PathBuf> = recorded
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .chain(default)
        .filter(|folder| folder.is_dir())
        .collect();
    distinct(&mut folders);
    folders
}

/// Removes repeats, ignoring case as Windows paths do.
fn distinct(folders: &mut Vec<PathBuf>) {
    let mut seen = Vec::new();
    folders.retain(|folder| {
        let key = folder.display().to_string().to_lowercase();
        let new = !seen.contains(&key);
        if new {
            seen.push(key);
        }
        new
    });
}

/// Steam's text files are normally UTF-8, but one odd byte shouldn't hide a library.
fn read_text(path: &Path) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// Deadlock's log in whichever Steam library holds it.
fn find_deadlock_log() -> Result<PathBuf, String> {
    let steam = steam_folders();
    if steam.is_empty() {
        return Err(
            "Steam wasn't found on this PC, so there's no game log to read. Picking a server still \
             works. To read a log from elsewhere, set the game log path in F2."
                .to_string(),
        );
    }
    let mut libraries = steam.clone();
    for folder in &steam {
        if let Some(vdf) = read_text(&folder.join(r"steamapps\libraryfolders.vdf")) {
            libraries.extend(analysis::libraries(&vdf).into_iter().map(PathBuf::from));
        }
    }
    distinct(&mut libraries);
    libraries
        .iter()
        .map(|library| {
            let apps = library.join("steamapps");
            let manifest = apps.join(format!("appmanifest_{DEADLOCK_APP}.acf"));
            let folder = read_text(&manifest)
                .and_then(|text| analysis::install_folder(&text))
                .unwrap_or_else(|| DEADLOCK_FOLDER.to_string());
            apps.join("common").join(folder).join(LOG_IN_GAME)
        })
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "Deadlock's console.log wasn't found. Add -condebug to its launch options and run \
             the game once. F1 has the steps. For another game, set the game log path in F2."
                .to_string()
        })
}

/// The game's log: the configured path if there is one, otherwise Deadlock's log.
pub fn find_log(configured: Option<&str>) -> Result<PathBuf, String> {
    let Some(path) = configured else {
        return find_deadlock_log();
    };
    let path = PathBuf::from(path);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "No log at {}. Check the game log path in F2.",
            path.display()
        ))
    }
}

/// Reads the log, or the end of a very large one, and describes the file.
pub fn read_log(configured: Option<&str>) -> Result<(String, Context), String> {
    let path = find_log(configured)?;
    let describe = |error: io::Error| format!("Can't read {}: {error}", path.display());
    let mut file = open_shared(&path).map_err(describe)?;
    let metadata = file.metadata().map_err(describe)?;
    let bytes = metadata.len();
    let start = bytes.saturating_sub(MAX_LOG_BYTES);
    file.seek(SeekFrom::Start(start)).map_err(describe)?;
    let mut raw = Vec::new();
    file.read_to_end(&mut raw).map_err(describe)?;
    let context = Context {
        path: path.display().to_string(),
        bytes,
        truncated: start > 0,
        age_secs: metadata
            .modified()
            .ok()
            .and_then(|written| written.elapsed().ok())
            .map(|age| age.as_secs()),
        ..Context::default()
    };
    Ok((String::from_utf8_lossy(&raw).into_owned(), context))
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

/// One ICMP echo, in milliseconds. A game's real path can differ, especially through a routing tool.
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
