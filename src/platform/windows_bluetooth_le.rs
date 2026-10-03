//! Association-endpoint enumeration. Only an explicit scan watches unpaired LE
//! devices; the long-lived watcher is restricted to already paired endpoints.
use super::SysValue;
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock}, time::{Duration, Instant}};
use windows::{core::{HSTRING, Interface, IInspectable, Result},
    Devices::{Bluetooth::BluetoothLEDevice, Enumeration::*},
    Foundation::{IPropertyValue, TypedEventHandler}};
use windows_collections::IIterable;

const MAX_DEVICES: usize = 256;
const NEARBY_LIFETIME: Duration = Duration::from_secs(120);
const SCAN_TIME: Duration = Duration::from_secs(8);

fn properties() -> IIterable<HSTRING> {
    vec![HSTRING::from("System.Devices.Aep.IsConnected"), HSTRING::from("System.Devices.Aep.DeviceAddress")].into()
}

fn display_name(name: &str, address: &str, id: &str) -> String {
    if !name.trim().is_empty() { return name.into(); }
    if !address.is_empty() { return format!("({address})"); }
    // Some advertisements never carry a name. Keep their native identities
    // distinguishable instead of presenting several identical blank rows.
    let tail: String = id.chars().rev().take(24).collect::<Vec<_>>().into_iter().rev().collect();
    format!("Bluetooth LE ({tail})")
}

#[derive(Clone, Debug)]
struct Row { id: String, name: String, paired: bool, pairable: bool, connected: Option<bool> }
impl Row {
    fn from_device(device: &DeviceInformation) -> Result<Self> {
        let pairing = device.Pairing()?;
        let connected = device.Properties()?.Lookup(&HSTRING::from("System.Devices.Aep.IsConnected"))
            .and_then(|value| value.cast::<IPropertyValue>()).and_then(|value| value.GetBoolean()).ok();
        let id = device.Id()?.to_string();
        let address = device.Properties()?.Lookup(&HSTRING::from("System.Devices.Aep.DeviceAddress"))
            .and_then(|value| value.cast::<IPropertyValue>()).and_then(|value| value.GetString()).map(|value| value.to_string()).unwrap_or_default();
        Ok(Self { name: display_name(&device.Name()?.to_string(), &address, &id), id,
            paired: pairing.IsPaired()?, pairable: pairing.CanPair()?, connected })
    }
    fn value(&self) -> SysValue {
        SysValue::Map(vec![
            ("id".into(), SysValue::Text(format!("ble:{}", self.id))),
            ("name".into(), SysValue::Text(if self.name.is_empty() { "Bluetooth LE".into() } else { self.name.clone() })),
            ("transport".into(), SysValue::Text("le".into())),
            ("current".into(), SysValue::Bool(self.connected == Some(true))),
            ("connection_known".into(), SysValue::Bool(self.connected.is_some())),
            ("paired".into(), SysValue::Bool(self.paired)),
            ("pairable".into(), SysValue::Bool(self.pairable && !self.paired)),
            // Pairing is not an audio or GATT-profile connection command.
            ("controllable".into(), SysValue::Bool(false)),
        ])
    }
}

#[derive(Default)]
struct Catalog {
    devices: BTreeMap<String, (DeviceInformation, Row)>,
    complete: bool,
    closed: bool,
    error: String,
}
impl Catalog {
    fn add(&mut self, device: &DeviceInformation) -> Result<()> {
        if self.closed { return Ok(()); }
        let row = Row::from_device(device)?;
        if self.devices.len() < MAX_DEVICES || self.devices.contains_key(&row.id) {
            self.devices.insert(row.id.clone(), (device.clone(), row));
        } else { self.error = format!("Bluetooth LE catalog reached {MAX_DEVICES} devices"); }
        Ok(())
    }
    fn update(&mut self, update: &DeviceInformationUpdate) -> Result<()> {
        if self.closed { return Ok(()); }
        if let Some((device, row)) = self.devices.get_mut(&update.Id()?.to_string()) {
            device.Update(update)?;
            *row = Row::from_device(device)?;
        }
        Ok(())
    }
}

struct Watch {
    watcher: DeviceWatcher,
    catalog: Arc<Mutex<Catalog>>,
    added: Option<i64>, updated: Option<i64>, removed: Option<i64>, complete: Option<i64>,
}
impl Watch {
    fn new(paired: bool) -> Result<Self> {
        let selector = BluetoothLEDevice::GetDeviceSelectorFromPairingState(paired)?;
        let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
            &selector, &properties(), DeviceInformationKind::AssociationEndpoint)?;
        let catalog = Arc::new(Mutex::new(Catalog::default()));
        let mut watch = Self { watcher, catalog, added: None, updated: None, removed: None, complete: None };
        let data = watch.catalog.clone();
        watch.added = Some(watch.watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(move |_, item| {
            if let Some(item) = item.as_ref() {
                let mut data = data.lock().unwrap();
                if let Err(error) = data.add(item) { data.error = error.to_string(); }
            }
            Ok(())
        }))?);
        let data = watch.catalog.clone();
        watch.updated = Some(watch.watcher.Updated(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, update| {
            if let Some(update) = update.as_ref() {
                let mut data = data.lock().unwrap();
                if let Err(error) = data.update(update) { data.error = error.to_string(); }
            }
            Ok(())
        }))?);
        let data = watch.catalog.clone();
        watch.removed = Some(watch.watcher.Removed(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, update| {
            if let Some(update) = update.as_ref() {
                match update.Id() {
                    Ok(id) => { data.lock().unwrap().devices.remove(&id.to_string()); },
                    Err(error) => data.lock().unwrap().error = error.to_string(),
                }
            }
            Ok(())
        }))?);
        let data = watch.catalog.clone();
        watch.complete = Some(watch.watcher.EnumerationCompleted(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(move |_, _| {
            data.lock().unwrap().complete = true;
            Ok(())
        }))?);
        watch.watcher.Start()?;
        Ok(watch)
    }
    fn rows(&self) -> std::result::Result<(Vec<Row>, bool, String), String> {
        let status = self.watcher.Status().map_err(|e| e.to_string())?;
        if status == DeviceWatcherStatus::Aborted { return Err("Windows aborted Bluetooth LE enumeration".into()); }
        let data = self.catalog.lock().unwrap();
        // A malformed endpoint or a full catalog must not hide the other rows
        // or repeatedly restart a healthy watcher. Only watcher failure is fatal.
        Ok((data.devices.values().map(|(_, row)| row.clone()).collect(), data.complete, data.error.clone()))
    }
    fn stop(&self) -> std::result::Result<(), String> {
        self.watcher.Stop().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.watcher.Status().map_err(|e| e.to_string())? {
                DeviceWatcherStatus::Stopped => return Ok(()),
                DeviceWatcherStatus::Aborted => return Err("Windows aborted Bluetooth LE discovery".into()),
                _ if Instant::now() >= deadline => return Err("Bluetooth LE watcher did not stop within two seconds".into()),
                _ => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.catalog.lock().unwrap().closed = true;
        let _ = self.watcher.Stop();
        if let Some(token) = self.added { let _ = self.watcher.RemoveAdded(token); }
        if let Some(token) = self.updated { let _ = self.watcher.RemoveUpdated(token); }
        if let Some(token) = self.removed { let _ = self.watcher.RemoveRemoved(token); }
        if let Some(token) = self.complete { let _ = self.watcher.RemoveEnumerationCompleted(token); }
    }
}

#[derive(Default)]
struct State {
    paired: Option<Watch>,
    nearby: BTreeMap<String, (Row, Instant)>,
    scan_warning: String,
    retry_at: Option<Instant>,
    error: String,
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

pub(super) fn read(enabled: bool) -> (Vec<SysValue>, bool, String) {
    let mut state = state().lock().unwrap();
    if !enabled {
        state.paired = None;
        state.nearby.clear();
        state.scan_warning.clear();
        state.retry_at = None;
        state.error.clear();
        return (Vec::new(), false, String::new());
    }
    let now = Instant::now();
    state.nearby.retain(|_, (_, seen)| now.duration_since(*seen) < NEARBY_LIFETIME);
    if state.paired.is_none() && state.retry_at.is_none_or(|retry| now >= retry) {
        match Watch::new(true) {
            Ok(watch) => { state.paired = Some(watch); state.error.clear(); },
            Err(error) => { state.error = error.to_string(); state.retry_at = Some(now + Duration::from_secs(10)); },
        }
    }
    let mut rows: BTreeMap<String, Row> = state.nearby.iter().map(|(id, (row, _))| (id.clone(), row.clone())).collect();
    let mut ready = false;
    let mut warning = state.scan_warning.clone();
    if let Some(watch) = &state.paired {
        match watch.rows() {
            Ok((paired, complete, partial_error)) => {
                ready = complete;
                for row in paired { rows.insert(row.id.clone(), row); }
                if !partial_error.is_empty() {
                    if !warning.is_empty() { warning.push_str("; "); }
                    warning.push_str(&partial_error);
                }
            },
            Err(error) => {
                state.error = error;
                state.paired = None;
                state.retry_at = Some(now + Duration::from_secs(10));
            },
        }
    }
    if !state.error.is_empty() {
        if !warning.is_empty() { warning.push_str("; "); }
        warning.push_str(&state.error);
    }
    (rows.values().map(Row::value).collect(), ready, warning)
}

pub(super) struct Scan { watch: Watch, started: Instant }
impl Scan {
    pub(super) fn start() -> std::result::Result<Self, String> {
        Ok(Self { watch: Watch::new(false).map_err(|e| e.to_string())?, started: Instant::now() })
    }
    pub(super) fn finish(self) -> std::result::Result<usize, String> {
        // Classic inquiry can run concurrently on the caller while this watcher
        // receives LE events. Do not use EnumerationCompleted as a scan deadline.
        while self.started.elapsed() < SCAN_TIME {
            self.watch.rows()?;
            std::thread::sleep(Duration::from_millis(50));
        }
        self.watch.stop()?;
        let (rows, _, warning) = self.watch.rows()?;
        let count = rows.len();
        let seen = Instant::now();
        let mut state = state().lock().unwrap();
        state.nearby = rows.into_iter().map(|row| (row.id.clone(), (row, seen))).collect();
        state.scan_warning = warning;
        Ok(count)
    }
}

fn pairing_status(status: DevicePairingResultStatus) -> std::result::Result<(), String> {
    let message = match status {
        DevicePairingResultStatus::Paired | DevicePairingResultStatus::AlreadyPaired => return Ok(()),
        DevicePairingResultStatus::PairingCanceled => "Bluetooth pairing was cancelled",
        DevicePairingResultStatus::AccessDenied => "Windows denied Bluetooth pairing access",
        DevicePairingResultStatus::AuthenticationTimeout => "Bluetooth authentication timed out",
        DevicePairingResultStatus::ConnectionRejected => "The Bluetooth device rejected the connection",
        DevicePairingResultStatus::NotReadyToPair => "The Bluetooth device is not ready to pair",
        DevicePairingResultStatus::OperationAlreadyInProgress => "Windows is already pairing this device",
        _ => return Err(format!("Windows Bluetooth pairing failed (status {})", status.0)),
    };
    Err(message.into())
}

pub(super) fn pair(id: &str) -> std::result::Result<(), String> {
    let known = {
        let state = state().lock().unwrap();
        state.nearby.get(id).is_some_and(|(_, seen)| seen.elapsed() < NEARBY_LIFETIME)
            || state.paired.as_ref().is_some_and(|watch| watch.catalog.lock().unwrap().devices.contains_key(id))
    };
    if !known { return Err("Bluetooth LE device is no longer in the catalog; scan again".into()); }
    let read = || DeviceInformation::CreateFromIdAsyncWithKindAndAdditionalProperties(
        &HSTRING::from(id), &properties(), DeviceInformationKind::AssociationEndpoint)?.join();
    let device = read().map_err(|e| e.to_string())?;
    let pairing = device.Pairing().map_err(|e| e.to_string())?;
    if pairing.IsPaired().map_err(|e| e.to_string())? { return Ok(()); }
    if !pairing.CanPair().map_err(|e| e.to_string())? { return Err("Windows reports that this Bluetooth LE device cannot pair".into()); }
    // Basic pairing delegates PIN/consent UI to Windows. Never accept a
    // ceremony automatically, nor substitute Settings for a pairing request.
    let result = pairing.PairAsync().and_then(|op| op.join()).map_err(|e| e.to_string())?;
    pairing_status(result.Status().map_err(|e| e.to_string())?)?;
    let confirmed = read().map_err(|e| e.to_string())?;
    let row = Row::from_device(&confirmed).map_err(|e| e.to_string())?;
    if !row.paired { return Err("Windows has not confirmed Bluetooth LE pairing".into()); }
    let mut state = state().lock().unwrap();
    state.nearby.remove(id);
    if let Some(watch) = &state.paired {
        watch.catalog.lock().unwrap().add(&confirmed).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsuccessful_pairing_statuses_are_not_accepted() {
        for status in 0..=20 {
            assert_eq!(pairing_status(DevicePairingResultStatus(status)).is_ok(), status == 0 || status == 3);
        }
    }
    #[test]
    fn fabricated_le_ids_never_open_a_pairing_dialog() {
        assert!(pair("not-a-discovered-device").unwrap_err().contains("scan again"));
    }
    #[test]
    fn unnamed_advertisements_keep_distinguishable_native_labels() {
        assert_eq!(display_name("Teclado 日本", "aa:bb", "id"), "Teclado 日本");
        assert_ne!(display_name("", "aa:bb:00", "id-1"), display_name(" ", "aa:bb:01", "id-2"));
        assert!(display_name("", "", "native-id-1").contains("native-id-1"));
    }
    #[test]
    #[ignore = "requires a Bluetooth radio; discovers nearby devices but never pairs or changes the radio"]
    fn live_le_watch_and_scan() {
        let _apartment = super::super::super::windows_system::Apartment::new().unwrap();
        let before = super::super::radios().unwrap();
        assert!(before.iter().any(|radio| radio.State().ok() == Some(windows::Devices::Radios::RadioState::On)), "enable a Bluetooth radio before opting into this test");
        let (_, _, error) = read(true);
        assert!(error.is_empty(), "{error}");
        let count = Scan::start().unwrap().finish().unwrap();
        let (rows, ready, error) = read(true);
        assert!(error.is_empty(), "{error}");
        eprintln!("LE watcher: paired enumeration complete={ready}; discovered={count}; catalog={}", rows.len());
        let paired_id = {
            let state = state().lock().unwrap();
            state.paired.as_ref().and_then(|watch| watch.catalog.lock().unwrap().devices.values()
                .find(|(_, row)| row.paired).map(|(_, row)| row.id.clone()))
        };
        if let Some(id) = paired_id {
            // This path re-reads an already paired identity and returns before
            // PairAsync. No new pairing ceremony or settings change is made.
            super::super::pair(&format!("ble:{id}")).unwrap();
            eprintln!("LE already-paired identity revalidation: passed");
        }
        assert!(state().lock().unwrap().nearby.values().all(|(_, seen)| seen.elapsed() < NEARBY_LIFETIME));
        read(false); // Release only the test's watchers; do not change the radio.
        assert!(state().lock().unwrap().paired.is_none());
        assert!(before.iter().any(|radio| radio.State().ok() == Some(windows::Devices::Radios::RadioState::On)));
    }
}
