//! Steam Datagram Relay config: fetching, parsing and the pick-to-blocklist calculation.

use crate::wide;
use miniserde::{Deserialize, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::Ipv4Addr;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Networking::WinHttp::*;
use windows_sys::w;

/// The API is per-app but apps share a few relay sets. Sampling popular titles found three:
/// CS2 (global plus China), Dota 2, and a default that Deadlock and most others use.
pub const APPS: [u32; 3] = [730, 570, 1422450];

static DECOMPRESSION_ALL: u32 = 3;
const MAX_BODY: usize = 8 << 20;

#[derive(Deserialize)]
struct RawConfig {
    pops: BTreeMap<String, RawPop>,
}

#[derive(Deserialize)]
struct RawPop {
    desc: Option<String>,
    relays: Option<Vec<RawRelay>>,
}

#[derive(Deserialize)]
struct RawRelay {
    ipv4: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct Pop {
    pub desc: String,
    pub ips: BTreeSet<Ipv4Addr>,
}

/// Relay sites keyed by Valve's PoP code, e.g. `fra`.
pub type Pops = BTreeMap<String, Pop>;

/// Relay addresses to block, grouped by PoP.
pub type Blocks<'a> = BTreeMap<&'a str, Vec<Ipv4Addr>>;

/// Ignores addresses that could never be a relay, so a bad config can't cut off the LAN.
fn routable(ip: &Ipv4Addr) -> bool {
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_unspecified())
}

/// Keeps only PoPs with usable relays; many entries are partner stubs with none.
pub fn parse(body: &str) -> Result<Pops, String> {
    let raw: RawConfig = json::from_str(body).map_err(|_| "malformed SDR config".to_string())?;
    let pops: Pops = raw
        .pops
        .into_iter()
        .filter_map(|(code, pop)| {
            let ips: BTreeSet<Ipv4Addr> = pop
                .relays?
                .iter()
                .filter_map(|relay| relay.ipv4.as_deref()?.parse().ok())
                .filter(routable)
                .collect();
            let desc = pop.desc.unwrap_or_else(|| code.clone());
            (!ips.is_empty()).then_some((code, Pop { desc, ips }))
        })
        .collect();
    if pops.is_empty() {
        return Err("SDR config has no relays".into());
    }
    Ok(pops)
}

/// Unions per-app configs so a block covers every game at once.
pub fn merge<'a>(configs: impl Iterator<Item = &'a Pops>) -> Pops {
    let mut merged = Pops::new();
    for (code, pop) in configs.flatten() {
        merged
            .entry(code.clone())
            .or_insert_with(|| Pop {
                desc: pop.desc.clone(),
                ips: BTreeSet::new(),
            })
            .ips
            .extend(&pop.ips);
    }
    merged
}

/// Every relay except the open sites'. An address they share with another PoP stays open. Nothing
/// is blocked if any open site is unknown, so bad data can't cut off the one you asked for.
pub fn blocklist<'a>(pops: &'a Pops, open: &[&str]) -> Blocks<'a> {
    let mut keep = BTreeSet::new();
    for code in open {
        let Some(pop) = pops.get(*code) else {
            return Blocks::new();
        };
        keep.extend(&pop.ips);
    }
    if keep.is_empty() {
        return Blocks::new();
    }
    pops.iter()
        .filter(|(code, _)| !open.contains(&code.as_str()))
        .filter_map(|(code, pop)| {
            let ips: Vec<_> = pop.ips.difference(&keep).copied().collect();
            (!ips.is_empty()).then_some((code.as_str(), ips))
        })
        .collect()
}

/// Identifies an applied state, so unchanged data never touches the filters. The open sites are
/// readable in it, which is how a reopened app recovers them.
pub fn stamp(open: &[&str], blocks: &Blocks) -> String {
    let mut hasher = DefaultHasher::new();
    blocks.hash(&mut hasher);
    format!("{} {:016x}", open.join("+"), hasher.finish())
}

/// The open sites a stamp names: the pick, and the backup if there is one.
pub fn sites_in_stamp(stamp: &str) -> (String, Option<String>) {
    let sites = stamp.split(' ').next().unwrap_or_default();
    match sites.split_once('+') {
        Some((pick, backup)) => (pick.to_string(), Some(backup.to_string())),
        None => (sites.to_string(), None),
    }
}

struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void, what: &str) -> Result<Self, String> {
        if raw.is_null() {
            return Err(last_error(what));
        }
        Ok(Self(raw))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn last_error(what: &str) -> String {
    format!("{what} failed: error {}", unsafe { GetLastError() })
}

/// WinHTTP client. Reusing it keeps the TLS connection alive between refreshes, and using the OS
/// stack means the system proxy and certificate store are honoured with no bundled TLS.
pub struct Client {
    // Declared first so it closes before the session it belongs to.
    connection: Handle,
    _session: Handle,
}

impl Client {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let session = Handle::new(
                WinHttpOpen(
                    w!("dl-conn-rs"),
                    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                    null(),
                    null(),
                    0,
                ),
                "WinHttpOpen",
            )?;
            WinHttpSetTimeouts(session.0, 5_000, 5_000, 5_000, 10_000);
            WinHttpSetOption(
                session.0,
                WINHTTP_OPTION_DECOMPRESSION,
                (&raw const DECOMPRESSION_ALL).cast(),
                4,
            );
            let connection = Handle::new(
                WinHttpConnect(session.0, w!("api.steampowered.com"), 443, 0),
                "WinHttpConnect",
            )?;
            Ok(Self {
                connection,
                _session: session,
            })
        }
    }

    pub fn config(&self, app: u32) -> Result<Pops, String> {
        parse(&self.get(&format!("/ISteamApps/GetSDRConfig/v1/?appid={app}"))?)
    }

    fn get(&self, path: &str) -> Result<String, String> {
        unsafe {
            let request = Handle::new(
                WinHttpOpenRequest(
                    self.connection.0,
                    w!("GET"),
                    wide(path).as_ptr(),
                    null(),
                    null(),
                    null(),
                    WINHTTP_FLAG_SECURE,
                ),
                "WinHttpOpenRequest",
            )?;
            if WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) == 0 {
                return Err(last_error("WinHttpSendRequest"));
            }
            if WinHttpReceiveResponse(request.0, null_mut()) == 0 {
                return Err(last_error("WinHttpReceiveResponse"));
            }
            let (mut status, mut size) = (0u32, 4u32);
            let query = WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER;
            let status_ptr = (&raw mut status).cast();
            if WinHttpQueryHeaders(request.0, query, null(), status_ptr, &mut size, null_mut()) == 0
            {
                return Err(last_error("WinHttpQueryHeaders"));
            }
            if status != 200 {
                return Err(format!("HTTP {status}"));
            }
            let mut body = Vec::new();
            loop {
                let mut available = 0;
                if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                    return Err(last_error("WinHttpQueryDataAvailable"));
                }
                if available == 0 {
                    break;
                }
                let start = body.len();
                if start + available as usize > MAX_BODY {
                    return Err("response too large".into());
                }
                body.resize(start + available as usize, 0);
                let mut read = 0;
                let target = body[start..].as_mut_ptr().cast();
                if WinHttpReadData(request.0, target, available, &mut read) == 0 {
                    return Err(last_error("WinHttpReadData"));
                }
                body.truncate(start + read as usize);
            }
            String::from_utf8(body).map_err(|_| "response is not UTF-8".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"revision":1,"success":true,"certs":["x"],"pops":{
        "fra":{"desc":"Frankfurt (Germany)","geo":[8.5,50.1],"partners":1,"tier":1,
            "relays":[{"ipv4":"155.133.226.75","port_range":[27015,27060]},
                      {"ipv4":"155.133.226.76","port_range":[27015,27060]}]},
        "waw":{"desc":"Warsaw (Poland)","relays":[{"ipv4":"155.133.230.34"},
                                                   {"ipv4":"155.133.226.76"}]},
        "ctu":{"desc":"Partner stub (China)","partners":2,"tier":1},
        "bad":{"desc":"Junk","relays":[{"ipv4":"not-an-ip"},{"ipv4":"192.168.1.1"},{}]}
    }}"#;

    fn pops() -> Pops {
        parse(SAMPLE).unwrap()
    }

    #[test]
    fn parse_keeps_only_pops_with_usable_relays() {
        let pops = pops();
        assert_eq!(pops.keys().collect::<Vec<_>>(), ["fra", "waw"]);
        assert_eq!(pops["fra"].desc, "Frankfurt (Germany)");
        assert_eq!(pops["fra"].ips.len(), 2);
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(parse("<html>").is_err());
        assert!(parse(r#"{"pops":{}}"#).is_err());
    }

    #[test]
    fn blocklist_never_blocks_an_address_the_pick_uses() {
        let pops = pops();
        let blocks = blocklist(&pops, &["fra"]);
        let shared: Ipv4Addr = "155.133.226.76".parse().unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(!blocks["waw"].contains(&shared));
        assert_eq!(
            blocks["waw"],
            ["155.133.230.34".parse::<Ipv4Addr>().unwrap()]
        );
    }

    #[test]
    fn unknown_pick_blocks_nothing() {
        assert!(blocklist(&pops(), &["nope"]).is_empty());
        assert!(blocklist(&pops(), &["fra", "nope"]).is_empty());
        assert!(blocklist(&pops(), &[]).is_empty());
    }

    #[test]
    fn merge_unions_relays_across_apps() {
        let a = parse(r#"{"pops":{"fra":{"desc":"F","relays":[{"ipv4":"1.1.1.1"}]}}}"#).unwrap();
        let b = parse(
            r#"{"pops":{"fra":{"desc":"F","relays":[{"ipv4":"2.2.2.2"}]},
                        "waw":{"desc":"W","relays":[{"ipv4":"3.3.3.3"}]}}}"#,
        )
        .unwrap();
        let merged = merge([&a, &b].into_iter());
        assert_eq!(merged.len(), 2);
        assert_eq!(merged["fra"].ips.len(), 2);
    }

    #[test]
    fn stamp_tracks_both_pick_and_relays() {
        let pops = pops();
        let blocks = blocklist(&pops, &["fra"]);
        let same = stamp(&["fra"], &blocks);
        assert_eq!(same, stamp(&["fra"], &blocklist(&pops, &["fra"])));
        assert_ne!(same, stamp(&["fra"], &Blocks::new()));
        assert_ne!(same, stamp(&["fra", "waw"], &blocks));
        assert!(same.starts_with("fra "));
    }

    #[test]
    fn a_backup_stays_open_too() {
        let pops = parse(
            r#"{"pops":{"fra":{"desc":"F","relays":[{"ipv4":"1.1.1.1"}]},
                        "waw":{"desc":"W","relays":[{"ipv4":"2.2.2.2"}]},
                        "jnb":{"desc":"J","relays":[{"ipv4":"3.3.3.3"}]}}}"#,
        )
        .unwrap();
        let blocks = blocklist(&pops, &["fra", "waw"]);
        assert_eq!(blocks.keys().copied().collect::<Vec<_>>(), ["jnb"]);
    }

    #[test]
    fn stamps_name_their_sites() {
        let none = (String::new(), None);
        assert_eq!(sites_in_stamp(""), none);
        assert_eq!(sites_in_stamp("fra 0123"), ("fra".to_string(), None));
        assert_eq!(
            sites_in_stamp("fra+waw 0123"),
            ("fra".to_string(), Some("waw".to_string()))
        );
        let blocks = Blocks::new();
        assert_eq!(
            sites_in_stamp(&stamp(&["fra", "waw"], &blocks)),
            ("fra".to_string(), Some("waw".to_string()))
        );
    }
}
