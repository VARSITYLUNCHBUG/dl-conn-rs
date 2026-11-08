//! Relay blocking through the Windows Filtering Platform.
//!
//! Filters are static WFP objects: not tied to this process and never written to disk, so they
//! outlive the app and vanish on reboot. The sublayer's description holds the applied stamp,
//! which makes WFP itself the only copy of our state.

use crate::sdr::Blocks;
use crate::{from_wide, wide};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::mem::zeroed;
use std::net::Ipv4Addr;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows_sys::core::GUID;

const SUBLAYER: GUID = GUID::from_u128(0x7c1d6e52_3f4a_4b8e_9a21_d15c0a7e4f10);
const SUBLAYER_WEIGHT: u16 = 0xFFFF;
const RPC_C_AUTHN_WINNT: u32 = 10;
const ENUM_BATCH: u32 = 256;

/// Where blocks are enforced. It runs once per flow, so it costs nothing per packet, and Windows
/// re-checks live flows when filters change, so blocks apply immediately. CONNECT_REDIRECT looks
/// like a better spot because it runs earlier, but block filters there are accepted and ignored.
const LAYER: GUID = FWPM_LAYER_ALE_AUTH_CONNECT_V4;

/// Dropped packets seen by [`Engine::watch_drops`], by remote address and app.
pub static DROPS: Mutex<BTreeMap<(Ipv4Addr, String), u32>> = Mutex::new(BTreeMap::new());

pub struct Engine {
    handle: HANDLE,
    watcher: Cell<HANDLE>,
}

// The raw handles are only ever used by the thread that owns the Engine.
unsafe impl Send for Engine {}

fn check(code: u32, what: &str) -> Result<(), String> {
    match code {
        0 => Ok(()),
        _ => Err(format!("{what} failed: {code:#010x}")),
    }
}

impl Engine {
    pub fn open() -> Result<Self, String> {
        let mut handle = null_mut();
        let code =
            unsafe { FwpmEngineOpen0(null(), RPC_C_AUTHN_WINNT, null(), null(), &mut handle) };
        check(code, "FwpmEngineOpen0")?;
        Ok(Self {
            handle,
            watcher: Cell::new(null_mut()),
        })
    }

    /// The applied stamp, if any block is active.
    pub fn stamp(&self) -> Option<String> {
        unsafe {
            let mut sublayer = null_mut();
            if FwpmSubLayerGetByKey0(self.handle, &SUBLAYER, &mut sublayer) != 0 {
                return None;
            }
            let text = (*sublayer).displayData.description;
            let len = (0..).take_while(|&i| *text.add(i) != 0).count();
            let stamp = from_wide(std::slice::from_raw_parts(text, len));
            FwpmFreeMemory0((&raw mut sublayer).cast());
            Some(stamp)
        }
    }

    /// Atomically replaces any current block.
    pub fn apply(&self, stamp: &str, blocks: &Blocks) -> Result<(), String> {
        self.transact(|| {
            self.remove_all()?;
            self.add_sublayer(stamp)?;
            blocks
                .iter()
                .try_for_each(|(pop, ips)| self.add_filter(pop, ips))
        })
    }

    pub fn clear(&self) -> Result<(), String> {
        self.transact(|| self.remove_all())
    }

    /// One transaction means no moment where relays are open or half-blocked mid-swap.
    fn transact(&self, work: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        unsafe {
            check(FwpmTransactionBegin0(self.handle, 0), "transaction begin")?;
            match work() {
                Ok(()) => check(FwpmTransactionCommit0(self.handle), "transaction commit"),
                Err(error) => {
                    FwpmTransactionAbort0(self.handle);
                    Err(error)
                }
            }
        }
    }

    fn remove_all(&self) -> Result<(), String> {
        if self.stamp().is_none() {
            return Ok(());
        }
        for id in self.filter_ids()? {
            check(
                unsafe { FwpmFilterDeleteById0(self.handle, id) },
                "filter delete",
            )?;
        }
        check(
            unsafe { FwpmSubLayerDeleteByKey0(self.handle, &SUBLAYER) },
            "sublayer delete",
        )
    }

    fn filter_ids(&self) -> Result<Vec<u64>, String> {
        unsafe {
            let mut template: FWPM_FILTER_ENUM_TEMPLATE0 = zeroed();
            template.layerKey = LAYER;
            template.enumType = FWP_FILTER_ENUM_OVERLAPPING;
            template.actionMask = u32::MAX;
            let mut enumerator = null_mut();
            check(
                FwpmFilterCreateEnumHandle0(self.handle, &template, &mut enumerator),
                "filter enum open",
            )?;
            let mut ids = Vec::new();
            let result = loop {
                let (mut entries, mut count) = (null_mut(), 0);
                let code = FwpmFilterEnum0(
                    self.handle,
                    enumerator,
                    ENUM_BATCH,
                    &mut entries,
                    &mut count,
                );
                if code != 0 {
                    break check(code, "filter enum");
                }
                if count > 0 {
                    let filters = std::slice::from_raw_parts(entries, count as usize);
                    for filter in filters.iter().map(|filter| &**filter) {
                        if same_guid(&filter.subLayerKey, &SUBLAYER) {
                            ids.push(filter.filterId);
                        }
                    }
                    FwpmFreeMemory0((&raw mut entries).cast());
                }
                if count < ENUM_BATCH {
                    break Ok(());
                }
            };
            FwpmFilterDestroyEnumHandle0(self.handle, enumerator);
            result.map(|()| ids)
        }
    }

    fn add_sublayer(&self, stamp: &str) -> Result<(), String> {
        let (mut name, mut description) = (wide("dl-conn-rs"), wide(stamp));
        unsafe {
            let mut sublayer: FWPM_SUBLAYER0 = zeroed();
            sublayer.subLayerKey = SUBLAYER;
            sublayer.displayData.name = name.as_mut_ptr();
            sublayer.displayData.description = description.as_mut_ptr();
            sublayer.weight = SUBLAYER_WEIGHT;
            check(
                FwpmSubLayerAdd0(self.handle, &sublayer, null_mut()),
                "sublayer add",
            )
        }
    }

    /// One filter per PoP. WFP ORs conditions on the same field, so this matches any of its
    /// relays. Hard block: clearing the action right stops lower-weight filters overriding it.
    fn add_filter(&self, pop: &str, ips: &[Ipv4Addr]) -> Result<(), String> {
        let mut name = wide(&format!("dl-conn-rs block {pop}"));
        unsafe {
            let mut conditions: Vec<FWPM_FILTER_CONDITION0> = ips
                .iter()
                .map(|ip| {
                    let mut condition: FWPM_FILTER_CONDITION0 = zeroed();
                    condition.fieldKey = FWPM_CONDITION_IP_REMOTE_ADDRESS;
                    condition.matchType = FWP_MATCH_EQUAL;
                    condition.conditionValue.r#type = FWP_UINT32;
                    condition.conditionValue.Anonymous.uint32 = u32::from(*ip);
                    condition
                })
                .collect();
            let mut filter: FWPM_FILTER0 = zeroed();
            filter.displayData.name = name.as_mut_ptr();
            filter.flags = FWPM_FILTER_FLAG_CLEAR_ACTION_RIGHT;
            filter.layerKey = LAYER;
            filter.subLayerKey = SUBLAYER;
            filter.weight.r#type = FWP_EMPTY;
            filter.numFilterConditions = conditions.len() as u32;
            filter.filterCondition = conditions.as_mut_ptr();
            filter.action.r#type = FWP_ACTION_BLOCK;
            check(
                FwpmFilterAdd0(self.handle, &filter, null_mut(), null_mut()),
                "filter add",
            )
        }
    }

    /// Starts or stops collecting drops. Only on while the console is open, so it costs nothing
    /// the rest of the time.
    pub fn watch_drops(&self, on: bool) -> Result<(), String> {
        let current = self.watcher.get();
        unsafe {
            if on && current.is_null() {
                let subscription: FWPM_NET_EVENT_SUBSCRIPTION0 = zeroed();
                let mut watcher = null_mut();
                check(
                    FwpmNetEventSubscribe0(
                        self.handle,
                        &subscription,
                        Some(on_net_event),
                        null(),
                        &mut watcher,
                    ),
                    "net event subscribe",
                )?;
                self.watcher.set(watcher);
            } else if !on && !current.is_null() {
                self.watcher.set(null_mut());
                check(
                    FwpmNetEventUnsubscribe0(self.handle, current),
                    "net event unsubscribe",
                )?;
            }
        }
        Ok(())
    }

    /// Whether any of our objects would survive a reboot. Must always be false.
    #[cfg(test)]
    pub fn has_persistent_objects(&self) -> bool {
        unsafe {
            let mut persistent = false;
            let mut sublayer = null_mut();
            if FwpmSubLayerGetByKey0(self.handle, &SUBLAYER, &mut sublayer) == 0 {
                persistent |= (*sublayer).flags & FWPM_SUBLAYER_FLAG_PERSISTENT != 0;
                FwpmFreeMemory0((&raw mut sublayer).cast());
            }
            for id in self.filter_ids().unwrap() {
                let mut filter = null_mut();
                assert_eq!(FwpmFilterGetById0(self.handle, id, &mut filter), 0);
                persistent |= (*filter).flags & FWPM_FILTER_FLAG_PERSISTENT != 0;
                FwpmFreeMemory0((&raw mut filter).cast());
            }
            persistent
        }
    }
}

unsafe extern "system" fn on_net_event(_context: *mut c_void, event: *const FWPM_NET_EVENT1) {
    // Requiring an app skips inbound replies to closed sockets, which aren't blocks.
    const NEEDED: u32 = FWPM_NET_EVENT_FLAG_REMOTE_ADDR_SET
        | FWPM_NET_EVENT_FLAG_IP_VERSION_SET
        | FWPM_NET_EVENT_FLAG_APP_ID_SET;
    let event = unsafe { &*event };
    let header = &event.header;
    if event.r#type != FWPM_NET_EVENT_TYPE_CLASSIFY_DROP
        || header.flags & NEEDED != NEEDED
        || header.ipVersion != FWP_IP_VERSION_V4
        || header.appId.data.is_null()
    {
        return;
    }
    let ip = Ipv4Addr::from(unsafe { header.Anonymous2.remoteAddrV4 });
    let units = header.appId.size as usize / 2;
    let path = unsafe { std::slice::from_raw_parts(header.appId.data.cast::<u16>(), units) };
    let path = from_wide(path);
    let app = path.rsplit('\\').next().unwrap_or_default().to_string();
    *DROPS.lock().unwrap().entry((ip, app)).or_default() += 1;
}

fn same_guid(a: &GUID, b: &GUID) -> bool {
    (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
}
