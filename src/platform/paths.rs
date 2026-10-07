//! File URIs use UTF-8 bytes and forward slashes on every system.
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
pub fn storage_component(name: &str) -> bool {
    // Win32 treats these as devices even with extensions; trailing dots alias
    // another file. Reject them before a successful write can discard data.
    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !name.ends_with('.') && !matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        && !(base.strip_prefix("COM").or_else(|| base.strip_prefix("LPT"))
            .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")))
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost/").map_or_else(|| rest.to_owned(), |p| format!("/{p}"));
    let mut decoded = Vec::new();
    let mut bytes = rest.as_bytes().iter().copied();
    while let Some(b) = bytes.next() {
        decoded.push(if b == b'%' {
            let hex = [bytes.next()?, bytes.next()?];
            u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?
        } else { b });
    }
    let path = String::from_utf8(decoded).ok()?;
    if path.contains('\0') { return None; }
    #[cfg(target_os = "windows")]
    {
        let path = if path.starts_with('/') && path.as_bytes().get(2) == Some(&b':') { path[1..].to_owned() }
            else if !path.starts_with('/') { format!("//{path}") } else { path };
        Some(PathBuf::from(path.replace('/', "\\")))
    }
    #[cfg(not(target_os = "windows"))]
    path.starts_with('/').then(|| PathBuf::from(path))
}

pub fn path_to_uri(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(target_os = "windows")]
    let path = {
        let path = path.replace('\\', "/");
        let path = path.strip_prefix("//?/UNC/").map(|p| format!("//{p}")).unwrap_or_else(|| path.strip_prefix("//?/").unwrap_or(&path).to_owned());
        if path.starts_with("//") { path[2..].to_owned() } else { format!("/{path}") }
    };
    let encoded: String = path.bytes().map(|b| {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) { (b as char).to_string() }
        else { format!("%{b:02X}") }
    }).collect();
    format!("file://{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_file_uri_roundtrip() {
        let path = if cfg!(windows) { PathBuf::from("C:\\escenas ñ\\日本 #1.plm") } else { PathBuf::from("/escenas ñ/日本 #1.plm") };
        let uri = path_to_uri(&path);
        assert!(uri.contains("%20") && uri.contains("%C3%B1") && uri.contains("%23"));
        assert_eq!(uri_to_path(&uri), Some(path));
        assert!(uri_to_path("file:///bad%zz").is_none());
        assert!(uri_to_path("file:///bad%00").is_none());
    }
    #[test]
    #[cfg(windows)]
    fn unc_and_extended_paths() {
        assert_eq!(uri_to_path("file://server/share/a%20b.plm"), Some(PathBuf::from(r"\\server\share\a b.plm")));
        assert_eq!(path_to_uri(Path::new(r"\\?\C:\a b.plm")), "file:///C:/a%20b.plm");
        assert_eq!(path_to_uri(Path::new(r"\\?\UNC\server\share\a.plm")), "file://server/share/a.plm");
    }
    #[test]
    #[cfg(windows)]
    fn scene_storage_rejects_windows_devices_and_aliases() {
        for name in ["NUL", "con.txt", "AUX.json", "COM1.log", "lpt9", "COM¹.txt", "settings."] {
            assert!(!storage_component(name), "{name}");
        }
        for name in ["settings.json", "COM10", "España", "世界"] { assert!(storage_component(name)); }
    }
}
