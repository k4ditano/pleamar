//! Explicit sharing of the currently connected WLAN profile. Never polled.
use super::{Allocation, Client, SysValue, checked, hex_ssid, valid_profile, wide_text};
use std::{cell::RefCell, fs::{File, OpenOptions}, os::windows::fs::OpenOptionsExt, path::{Path, PathBuf}};
use windows::{core::{PCWSTR, PWSTR}, Win32::{Foundation::{GENERIC_READ, GENERIC_WRITE},
    NetworkManagement::WiFi::*, Storage::FileSystem::*}};
use zeroize::{Zeroize, Zeroizing};
use image::ImageEncoder;

struct SharedQr { _file: File, path: PathBuf }
thread_local! { static QR: RefCell<Option<SharedQr>> = const { RefCell::new(None) }; }
pub(super) fn clear() { QR.with(|qr| { qr.borrow_mut().take(); }); }
pub(super) fn has_thread_state() -> bool { QR.with(|qr| qr.borrow().is_some()) }

struct Profile { name: String, password: Zeroizing<String>, kind: &'static str, hidden: bool }
fn field<'a, 'input>(parent: roxmltree::Node<'a, 'input>, name: &str) -> Result<roxmltree::Node<'a, 'input>, String> {
    let mut children = parent.children().filter(|n| n.has_tag_name(name));
    let value = children.next().ok_or_else(|| format!("Wi-Fi profile is missing {name}"))?;
    if children.next().is_some() { return Err(format!("Wi-Fi profile repeats {name}")); }
    Ok(value)
}
fn text<'a, 'input>(parent: roxmltree::Node<'a, 'input>, name: &str) -> Result<&'a str, String> {
    Ok(field(parent, name)?.text().unwrap_or(""))
}
fn parse(xml: &str, expected_ssid: &str) -> Result<Profile, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|_| "Windows returned an invalid Wi-Fi profile")?;
    let root = doc.root_element();
    if !root.has_tag_name("WLANProfile") { return Err("Windows returned an invalid Wi-Fi profile".into()); }
    let config = field(root, "SSIDConfig")?;
    let ssid = field(config, "SSID")?;
    let bytes = if ssid.children().any(|n| n.has_tag_name("hex")) {
        let hex = text(ssid, "hex")?;
        if hex.is_empty() || hex.len() > 64 || hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Wi-Fi profile has an invalid network identifier".into());
        }
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect::<Vec<_>>()
    } else { text(ssid, "name")?.as_bytes().to_vec() };
    let actual: String = bytes.iter().map(|v| format!("{v:02X}")).collect();
    if !actual.eq_ignore_ascii_case(expected_ssid) { return Err("The connected Wi-Fi network changed; open sharing again".into()); }
    let name = String::from_utf8(bytes).map_err(|_| "This network's binary SSID cannot be shared as a phone QR code")?;
    if name.is_empty() || name.len() > 32 || name.chars().any(char::is_control) {
        return Err("This network name cannot be shared as a phone QR code".into());
    }
    let security = field(field(root, "MSM")?, "security")?;
    let auth = field(security, "authEncryption")?;
    let encryption = text(auth, "encryption")?;
    let kind = match text(auth, "authentication")? {
        "open" if encryption == "none" => "nopass",
        "WPAPSK" | "WPA2PSK" => "WPA",
        "WPA3SAE" => "SAE",
        _ => return Err("This network uses enterprise or unsupported authentication; it has no shareable personal password".into()),
    };
    if text(auth, "useOneX").is_ok_and(|v| v != "false") {
        return Err("Enterprise Wi-Fi credentials cannot be shared as a personal network".into());
    }
    let password = if kind == "nopass" { String::new() } else {
        let key = field(security, "sharedKey")?;
        if text(key, "protected")? != "false" {
            return Err("Windows does not allow this application to read the saved Wi-Fi password".into());
        }
        let password = text(key, "keyMaterial")?;
        if password.is_empty() || password.len() > 128 || password.contains('\0') {
            return Err("Windows returned an invalid Wi-Fi password".into());
        }
        password.to_owned()
    };
    Ok(Profile { name, password: Zeroizing::new(password), kind,
        hidden: text(config, "nonBroadcast").is_ok_and(|v| v == "true") })
}
fn escape(value: &str) -> String {
    let mut result = String::new();
    for c in value.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') { result.push('\\'); }
        result.push(c);
    }
    result
}
fn payload(profile: &Profile) -> Zeroizing<String> {
    let escaped = Zeroizing::new(escape(&profile.password));
    Zeroizing::new(format!("WIFI:T:{};S:{};P:{};H:{};;", profile.kind, escape(&profile.name), escaped.as_str(), profile.hidden))
}
fn qr_image(profile: &Profile) -> Result<image::GrayImage, String> {
    let payload = payload(profile);
    let qr = qrcodegen::QrCode::encode_text(&payload, qrcodegen::QrCodeEcc::Medium)
        .map_err(|_| "The Wi-Fi details are too long for a QR code")?;
    let side = (qr.size() + 8) as u32 * 4;
    Ok(image::GrayImage::from_fn(side, side, |x, y| {
        image::Luma([if qr.get_module(x as i32 / 4 - 4, y as i32 / 4 - 4) { 0 } else { 255 }])
    }))
}
fn save_qr(profile: &Profile, directory: &Path) -> Result<SharedQr, String> {
    std::fs::create_dir_all(directory).map_err(|_| "Could not create the temporary Wi-Fi image directory")?;
    let id = unsafe { windows::Win32::System::Com::CoCreateGuid() }.map_err(|_| "Could not create a temporary Wi-Fi image name")?;
    let path = directory.join(format!("qr-{}-{id:?}.png", std::process::id()));
    let mut file = OpenOptions::new().read(true).write(true).create_new(true)
        .access_mode(GENERIC_READ.0 | GENERIC_WRITE.0 | DELETE.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_ATTRIBUTE_TEMPORARY.0 | FILE_FLAG_DELETE_ON_CLOSE.0)
        .open(&path).map_err(|_| "Could not create the temporary Wi-Fi image")?;
    // The OS deletes the PNG when this worker closes, including process exit.
    // No persistent password file, shell command or filename contains the key.
    let mut pixels = qr_image(profile)?;
    let result = image::codecs::png::PngEncoder::new(&mut file)
        .write_image(pixels.as_raw(), pixels.width(), pixels.height(), image::ExtendedColorType::L8);
    let bytes: &mut [u8] = pixels.as_mut();
    bytes.zeroize();
    result.map_err(|_| "Could not write the temporary Wi-Fi image")?;
    Ok(SharedQr { _file: file, path })
}
pub(super) fn read(args: &[SysValue]) -> Result<SysValue, String> {
    clear();
    let [SysValue::Text(interface), SysValue::Text(profile)] = args else { return Err("network.share takes the current interface and profile identifiers".into()); };
    if interface.is_empty() || interface.len() > 64 || !valid_profile(profile) { return Err("Invalid Wi-Fi sharing identifiers".into()); }
    let client = Client::open()?;
    let id = client.interfaces()?.into_iter().find(|i| format!("{:?}", i.InterfaceGuid) == *interface)
        .ok_or("The Wi-Fi adapter was removed")?.InterfaceGuid;
    let connection = client.connection(&id)?.ok_or("There is no connected Wi-Fi network to share")?;
    if connection.isState != wlan_interface_state_connected || wide_text(&connection.strProfileName) != *profile {
        return Err("The connected Wi-Fi profile changed; open sharing again".into());
    }
    let ssid = hex_ssid(&connection.wlanAssociationAttributes.dot11Ssid);
    let wide: Vec<_> = profile.encode_utf16().chain([0]).collect();
    let mut raw = PWSTR::null();
    let mut flags = WLAN_PROFILE_GET_PLAINTEXT_KEY;
    checked(unsafe { WlanGetProfile(client.0, &id, PCWSTR(wide.as_ptr()), None, &mut raw, Some(&mut flags), None) })?;
    let _allocation = Allocation(raw.0.cast());
    if raw.is_null() { return Err("Windows returned an empty Wi-Fi profile".into()); }
    let length = unsafe { raw.as_wide().len() };
    let xml = unsafe { raw.to_string() }.map(Zeroizing::new);
    for i in 0..length { unsafe { raw.0.add(i).write_volatile(0); } }
    let xml = xml.map_err(|_| "Windows returned invalid text in the Wi-Fi profile")?;
    let details = parse(&xml, &ssid)?;
    let current = client.connection(&id)?.ok_or("The Wi-Fi connection ended while sharing")?;
    if current.isState != wlan_interface_state_connected || wide_text(&current.strProfileName) != *profile
        || hex_ssid(&current.wlanAssociationAttributes.dot11Ssid) != ssid {
        return Err("The Wi-Fi connection changed while sharing".into());
    }
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
    let qr = save_qr(&details, &PathBuf::from(base).join("pleamar/wifi-share"))?;
    let path = qr.path.to_string_lossy().into_owned();
    QR.with(|slot| *slot.borrow_mut() = Some(qr));
    Ok(SysValue::Map(vec![
        ("name".into(), SysValue::Text(details.name)),
        ("password".into(), SysValue::Text(details.password.to_string())),
        ("open".into(), SysValue::Bool(details.kind == "nopass")),
        ("qr".into(), SysValue::Text(path)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    const SSID: &str = "436173613BC391";
    fn profile() -> String { super::super::personal_profile(SSID, "secret;,:\\<&>\"", true).unwrap().1 }
    fn decoded(image: &image::GrayImage) -> Vec<u8> {
        let mut decoder = quircs::Quirc::default();
        let mut codes = decoder.identify(image.width() as usize, image.height() as usize, image.as_raw());
        let value = codes.next().unwrap().unwrap().decode().unwrap().payload;
        assert!(codes.next().is_none());
        value
    }
    #[test]
    fn wireless_qr_roundtrip_keeps_unicode_and_escaped_credentials() {
        let xml = profile().replace("<SSIDConfig>", "<SSIDConfig><nonBroadcast>true</nonBroadcast>");
        let shared = parse(&xml, SSID).unwrap();
        assert_eq!(shared.name, "Casa;Ñ");
        assert_eq!(shared.password.as_str(), "secret;,:\\<&>\"");
        let expected = "WIFI:T:WPA;S:Casa\\;Ñ;P:secret\\;\\,\\:\\\\<&>\\\";H:true;;";
        assert_eq!(payload(&shared).as_str(), expected);
        assert_eq!(decoded(&qr_image(&shared).unwrap()), expected.as_bytes());
        let open = super::super::personal_profile(SSID, "", false).unwrap().1;
        let open = parse(&open, SSID).unwrap();
        assert_eq!(decoded(&qr_image(&open).unwrap()), "WIFI:T:nopass;S:Casa\\;Ñ;P:;H:false;;".as_bytes());
    }
    #[test]
    fn encrypted_enterprise_binary_and_changed_profiles_are_not_shared() {
        let xml = profile();
        let protected = xml.replace("<protected>false", "<protected>true");
        let error = parse(&protected, SSID).err().unwrap();
        assert!(error.contains("does not allow") && !error.contains("secret"));
        assert!(parse(&xml.replace("WPA2PSK", "WPA2"), SSID).is_err());
        assert!(parse(&xml.replace("<useOneX>false", "<useOneX>true"), SSID).is_err());
        assert!(parse(&xml, "00").is_err());
        assert!(parse(&xml.replace(SSID, "00FF"), "00FF").is_err());
        assert!(parse(&xml.replace("</hex>", "</hex><hex>00FF</hex><name>Casa;Ñ</name>"), SSID).is_err());
        assert!(parse(&xml.replace("<protected>false</protected>", "<protected>false</protected><protected>true</protected>"), SSID).is_err());
        assert!(parse("<!DOCTYPE x [<!ENTITY a SYSTEM 'file:///private'>]><x>&a;</x>", SSID).is_err());
        for args in [vec![], vec![SysValue::Text("name".into())], vec![SysValue::Text("id".into()), SysValue::Text("bad\0profile".into())]] {
            assert!(read(&args).is_err());
        }
    }
    #[test]
    fn temporary_qr_is_readable_until_cleared_and_removed_on_worker_exit() {
        let directory = std::env::temp_dir().join(format!("pleamar wifi ñ {:?}", unsafe { windows::Win32::System::Com::CoCreateGuid() }.unwrap()));
        let shared = parse(&profile(), SSID).unwrap();
        let qr = save_qr(&shared, &directory).unwrap();
        let path = qr.path.clone();
        let read = image::open(&path).unwrap().into_luma8();
        assert_eq!(decoded(&read), payload(&shared).as_bytes());
        QR.with(|slot| *slot.borrow_mut() = Some(qr));
        assert!(has_thread_state(), "the displayed credential image must pin its service thread");
        clear();
        assert!(!has_thread_state());
        assert!(!path.exists(), "closing sharing retained its credential image");
        let dir = directory.clone();
        let path = std::thread::spawn(move || {
            let qr = save_qr(&parse(&profile(), SSID).unwrap(), &dir).unwrap();
            let path = qr.path.clone();
            QR.with(|slot| *slot.borrow_mut() = Some(qr));
            path
        }).join().unwrap();
        assert!(!path.exists(), "worker exit retained its credential image");
        std::fs::remove_dir(directory).unwrap();
    }
}
