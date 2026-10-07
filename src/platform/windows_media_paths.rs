//! A host application may name its media folder without branding the backend.
use std::path::PathBuf;
use windows::{core::{Error, GUID, Result}, Win32::{Foundation::E_INVALIDARG,
    System::Com::CoTaskMemFree, UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT}}};

fn validated_name(value: &str) -> Result<String> {
    if value.is_empty() || value.encode_utf16().count() > 80 || value.ends_with([' ', '.'])
        || !super::paths::storage_component(value.split('.').next().unwrap_or("").trim_end_matches(' '))
        || value.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c)) {
        return Err(Error::new(E_INVALIDARG, "PLEAMAR_MEDIA_NAME must be a single valid Windows folder name (1–80 UTF-16 units)"));
    }
    Ok(value.to_owned())
}

pub fn name() -> Result<String> {
    match std::env::var("PLEAMAR_MEDIA_NAME") {
        Ok(value) => validated_name(&value),
        Err(std::env::VarError::NotPresent) => Ok("Pleamar".into()),
        Err(_) => Err(Error::new(E_INVALIDARG, "PLEAMAR_MEDIA_NAME is not valid Unicode")),
    }
}

pub fn folder(id: &GUID) -> Result<PathBuf> {
    let name = name()?;
    let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)? };
    let path = unsafe { raw.to_string() };
    unsafe { CoTaskMemFree(Some(raw.0 as _)); }
    Ok(PathBuf::from(path?).join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_name_stays_inside_the_known_folder() {
        for invalid in ["", ".", "..", "../captures", "C:\\pictures", "\\server", "a/b", "a:b",
            "a\0b", "a\nb", "name.", "name ", "NUL", "CON.txt", "CON .txt", "LPT¹", "a*", "a?", "a|", "a<", "a>", "a\""] {
            assert!(validated_name(invalid).is_err(), "{invalid:?}");
        }
        assert!(validated_name(&"🚀".repeat(41)).is_err());
        for valid in ["Pleamar", "My captures", "España 海 🚀", "COM10", "apps.backup"] {
            assert_eq!(validated_name(valid).unwrap(), valid);
        }
        assert!(validated_name(&"🚀".repeat(40)).is_ok());
    }

    #[test]
    fn configured_media_directory_uses_native_known_folders() {
        use windows::Win32::UI::Shell::{FOLDERID_Pictures, FOLDERID_Videos};
        for id in [&FOLDERID_Pictures, &FOLDERID_Videos] {
            match name() {
                Ok(name) => {
                    let directory = folder(id).unwrap();
                    assert!(directory.is_absolute());
                    assert_eq!(directory.file_name().unwrap(), name.as_str());
                    let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).unwrap() };
                    let native = unsafe { raw.to_string().unwrap() };
                    unsafe { CoTaskMemFree(Some(raw.0 as _)); }
                    assert_eq!(directory.parent().unwrap(), std::path::Path::new(&native));
                }
                Err(_) => assert!(folder(id).is_err()),
            }
        }
    }
}
