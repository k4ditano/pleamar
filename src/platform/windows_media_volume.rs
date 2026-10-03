//! Per-player Core Audio volume. Never substitutes the endpoint master volume.
use std::collections::{HashMap, HashSet};
use windows::{core::{Interface, PWSTR, GUID}, Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, PROPERTYKEY},
    Media::Audio::*,
    Storage::Packaging::Appx::GetApplicationUserModelId,
    System::{Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL,
        StructuredStorage::{PropVariantClear, PropVariantToStringAlloc}},
        Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_WIN32}},
    UI::{WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId},
        Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow}},
}};

const APP_ID: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

fn process_identity(pid: u32) -> Option<(String, String)> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut length);
        let mut id_length = 0;
        let _ = GetApplicationUserModelId(process, &mut id_length, None);
        let mut id = vec![0u16; (id_length as usize).min(32768)];
        id_length = id.len() as u32;
        let aumid = if !id.is_empty() && GetApplicationUserModelId(process, &mut id_length, Some(PWSTR(id.as_mut_ptr()))).is_ok() {
            String::from_utf16_lossy(&id[..id_length.saturating_sub(1) as usize])
        } else { String::new() };
        let _ = CloseHandle(process);
        result.ok()?;
        Some((String::from_utf16_lossy(&path[..length as usize]).to_lowercase(), aumid))
    }
}

struct WindowApps<'a> { player: &'a str, paths: HashSet<String> }
unsafe extern "system" fn window_app(hwnd: HWND, arg: LPARAM) -> windows::core::BOOL {
    unsafe {
        let apps = &mut *(arg.0 as *mut WindowApps<'_>);
        let read = || -> windows::core::Result<String> {
            let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd)?;
            let mut value = store.GetValue(&APP_ID)?;
            let raw = PropVariantToStringAlloc(&value);
            let _ = PropVariantClear(&mut value);
            let raw = raw?;
            let value = raw.to_string();
            CoTaskMemFree(Some(raw.0 as _));
            Ok(value?)
        };
        if read().is_ok_and(|id| !id.is_empty() && id == apps.player) {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if let Some((path, _)) = process_identity(pid) { apps.paths.insert(path); }
        }
        true.into()
    }
}

fn executable_matches(path: &str, player: &str) -> bool {
    let player = player.to_lowercase();
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    // Classic SMTC clients commonly publish their executable name. Full
    // packaged IDs must match exactly; a substring could control another app.
    player == path || player == name || name.strip_suffix(".exe").is_some_and(|stem| player == stem)
}

fn sessions(player: &str) -> Result<Vec<ISimpleAudioVolume>, String> {
    if player.is_empty() || player.contains('\0') { return Err("invalid media player identity".into()); }
    unsafe {
        let mut apps = WindowApps { player, paths: HashSet::new() };
        let _ = EnumWindows(Some(window_app), LPARAM(&mut apps as *mut _ as isize));
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).map_err(|e| e.to_string())?;
        let mut matched = HashMap::new();
        let mut volumes = Vec::new();
        for d in 0..devices.GetCount().map_err(|e| e.to_string())? {
            let mut read = || -> windows::core::Result<()> {
                let manager: IAudioSessionManager2 = devices.Item(d)?.Activate(CLSCTX_ALL, None)?;
                let sessions = manager.GetSessionEnumerator()?;
                for i in 0..sessions.GetCount()? {
                    let control: IAudioSessionControl2 = sessions.GetSession(i)?.cast()?;
                    if control.GetState()? == AudioSessionStateExpired { continue; }
                    let pid = control.GetProcessId()?;
                    if pid == 0 { continue; } // System sounds are never a media player.
                    let matches = *matched.entry(pid).or_insert_with(|| process_identity(pid).is_some_and(|(path, id)| {
                        id == player || apps.paths.contains(&path) || executable_matches(&path, player)
                    }));
                    if matches { volumes.push(control.cast::<ISimpleAudioVolume>()?); }
                }
                Ok(())
            };
            // A disconnected endpoint must not hide another working output.
            let _ = read();
        }
        if volumes.is_empty() { Err("The player has no identifiable audio session".into()) } else { Ok(volumes) }
    }
}

pub fn read(player: &str) -> Result<f64, String> {
    let mut level: f32 = 0.0;
    for volume in sessions(player)? {
        let current = unsafe { volume.GetMasterVolume() }.map_err(|e| e.to_string())?;
        let muted = unsafe { volume.GetMute() }.map_err(|e| e.to_string())?.as_bool();
        level = level.max(if muted { 0.0 } else { current });
    }
    Ok(level as f64)
}

pub fn set(player: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) { return Err("media volume must be between 0 and 1".into()); }
    let sessions = sessions(player)?;
    let previous: Result<Vec<_>, _> = sessions.iter().map(|v| unsafe {
        Ok::<_, windows::core::Error>((v.GetMasterVolume()?, v.GetMute()?.as_bool()))
    }).collect();
    let previous = previous.map_err(|e| e.to_string())?;
    for (i, volume) in sessions.iter().enumerate() {
        let result = unsafe { volume.SetMasterVolume(value as f32, std::ptr::null())
            .and_then(|_| volume.SetMute(false, std::ptr::null())) };
        if let Err(error) = result {
            for (volume, &(level, muted)) in sessions[..=i].iter().zip(&previous) {
                unsafe { let _ = volume.SetMasterVolume(level, std::ptr::null()); let _ = volume.SetMute(muted, std::ptr::null()); }
            }
            return Err(error.to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_do_not_match_other_players_and_invalid_levels_do_not_touch_audio() {
        assert!(executable_matches("c:\\apps\\spotify.exe", "Spotify.exe"));
        assert!(!executable_matches("c:\\apps\\spotify-helper.exe", "Spotify"));
        assert!(!executable_matches("c:\\apps\\spotify.exe", "Spotify_abcdef!App"));
        for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] { assert!(set("unused", value).is_err()); }
    }
}
