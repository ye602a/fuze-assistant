//! Explicit current-user ARP registration. No machine-wide keys, credentials or runtime polling.
use crate::{managed_install as install, Result};
use std::{
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS},
    System::Registry::*,
};
const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\FuzeRustPreview.Native";
const OWNER: &str = "fuze-rust-preview-v1";
const NAMES: &[&str] = &[
    "FuzeOwner",
    "DisplayName",
    "DisplayVersion",
    "Publisher",
    "InstallLocation",
    "UninstallString",
    "DisplayIcon",
    "NoModify",
    "NoRepair",
];
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}
fn open(name: &str) -> Result<Option<Key>> {
    let mut key = null_mut();
    let code = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(name).as_ptr(),
            REG_OPTION_OPEN_LINK,
            KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
            &mut key,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        Ok(None)
    } else if code == 0 {
        Ok(Some(Key(key)))
    } else {
        Err("registration-open")
    }
}
fn set(key: &Key, name: &str, kind: u32, bytes: &[u8]) -> Result<()> {
    if unsafe {
        RegSetValueExW(
            key.0,
            wide(name).as_ptr(),
            0,
            kind,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    } != 0
    {
        return Err("registration-write");
    }
    Ok(())
}
fn string(key: &Key, name: &str, value: &str) -> Result<()> {
    let bytes: Vec<_> = wide(value).into_iter().flat_map(u16::to_le_bytes).collect();
    set(key, name, REG_SZ, &bytes)
}
fn read(key: &Key, name: &str) -> Result<String> {
    let mut bytes = [0u8; 8192];
    let mut size = bytes.len() as u32;
    let mut kind = 0;
    if unsafe {
        RegQueryValueExW(
            key.0,
            wide(name).as_ptr(),
            null(),
            &mut kind,
            bytes.as_mut_ptr(),
            &mut size,
        )
    } != 0
        || kind != REG_SZ
        || size < 2
        || size % 2 != 0
    {
        return Err("registration-value");
    }
    let words: Vec<_> = bytes[..size as usize]
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    if words.last() != Some(&0) || words[..words.len() - 1].contains(&0) {
        return Err("registration-value");
    }
    String::from_utf16(&words[..words.len() - 1]).map_err(|_| "registration-value")
}
fn location(root: &Path) -> Result<String> {
    let value = std::fs::canonicalize(root).map_err(|_| "registration-location")?;
    let value = value.to_str().ok_or("registration-location")?;
    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    if value.len() > 240
        || value.contains(['"', '\r', '\n'])
        || value.as_bytes().get(1) != Some(&b':')
    {
        return Err("registration-location");
    }
    Ok(value.into())
}
fn command(location: &str) -> String {
    format!("\"{location}\\fuze-maintenance.exe\" --uninstall-local")
}
fn validate(key: &Key, root: &Path) -> Result<()> {
    let path = location(root)?;
    if read(key, "FuzeOwner")? != OWNER
        || read(key, "InstallLocation")? != path
        || read(key, "UninstallString")? != command(&path)
    {
        return Err("registration-owner-conflict");
    }
    for index in 0..=NAMES.len() {
        let mut name = [0u16; 128];
        let mut size = name.len() as u32;
        let code = unsafe {
            RegEnumValueW(
                key.0,
                index as u32,
                name.as_mut_ptr(),
                &mut size,
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if code == ERROR_NO_MORE_ITEMS {
            break;
        }
        if code != 0 || !NAMES.contains(&String::from_utf16_lossy(&name[..size as usize]).as_str())
        {
            return Err("registration-unknown-value");
        }
    }
    let mut subkeys = 0;
    if unsafe {
        RegQueryInfoKeyW(
            key.0,
            null_mut(),
            null_mut(),
            null(),
            &mut subkeys,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        )
    } != 0
        || subkeys != 0
    {
        return Err("registration-unknown-key");
    }
    Ok(())
}
pub fn preflight(root: &Path) -> Result<()> {
    if let Some(key) = open(KEY)? {
        validate(&key, root)?;
    }
    Ok(())
}
pub fn register(root: &Path) -> Result<()> {
    register_at(KEY, root, false)
}
pub fn refresh(root: &Path) -> Result<()> {
    if open(KEY)?.is_some() {
        register(root)?;
    }
    Ok(())
}
fn register_at(name: &str, root: &Path, volatile: bool) -> Result<()> {
    let path = location(root)?;
    let state = install::read(root)?;
    let version = state.current.ok_or("registration-no-version")?;
    crate::local_setup::regular(&root.join("fuze-maintenance.exe"))?;
    let key = if let Some(key) = open(name)? {
        validate(&key, root)?;
        key
    } else {
        let mut handle = null_mut();
        let mut disposition = 0;
        if unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(name).as_ptr(),
                0,
                null(),
                if volatile {
                    REG_OPTION_VOLATILE
                } else {
                    REG_OPTION_NON_VOLATILE
                },
                KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
                null(),
                &mut handle,
                &mut disposition,
            )
        } != 0
        {
            return Err("registration-create");
        }
        let key = Key(handle);
        if disposition != REG_CREATED_NEW_KEY {
            validate(&key, root)?;
        } else {
            // A failed initial registration is removed only from this newly created key.
            let initial = string(&key, "FuzeOwner", OWNER)
                .and_then(|_| string(&key, "InstallLocation", &path))
                .and_then(|_| string(&key, "UninstallString", &command(&path)));
            if initial.is_err() {
                drop(key);
                unsafe {
                    RegDeleteKeyExW(HKEY_CURRENT_USER, wide(name).as_ptr(), KEY_WOW64_64KEY, 0);
                }
                return initial;
            }
        }
        key
    };
    string(&key, "DisplayName", "Fuze Rust Preview")?;
    string(
        &key,
        "DisplayVersion",
        &format!(
            "{}{}",
            version.revision,
            if state.pending {
                " (pending validation)"
            } else {
                ""
            }
        ),
    )?;
    string(&key, "Publisher", "FUZE Project")?;
    string(
        &key,
        "DisplayIcon",
        &format!("\"{path}\\fuze-launcher.exe\",0"),
    )?;
    for field in ["NoModify", "NoRepair"] {
        set(&key, field, REG_DWORD, &1u32.to_le_bytes())?;
    }
    Ok(())
}
pub fn remove(root: &Path) -> Result<()> {
    remove_at(KEY, root)
}
fn remove_at(name: &str, root: &Path) -> Result<()> {
    if let Some(key) = open(name)? {
        validate(&key, root)?;
        drop(key);
        if unsafe { RegDeleteKeyExW(HKEY_CURRENT_USER, wide(name).as_ptr(), KEY_WOW64_64KEY, 0) }
            != 0
        {
            return Err("registration-remove");
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_registry_roundtrip_and_ownership_protection() {
        let id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("fuze-registration-{id}"));
        std::fs::create_dir(&root).unwrap();
        let name = format!(r"Software\FuzeRustRegistrationTest-{id}");
        struct Cleanup(String, std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe {
                    RegDeleteKeyExW(
                        HKEY_CURRENT_USER,
                        wide(&self.0).as_ptr(),
                        KEY_WOW64_64KEY,
                        0,
                    );
                }
                let _ = std::fs::remove_file(self.1.join("fuze-maintenance.exe"));
                let _ = std::fs::remove_file(self.1.join("current.json"));
                let _ = std::fs::remove_dir(&self.1);
            }
        }
        let _cleanup = Cleanup(name.clone(), root.clone());
        std::fs::write(root.join("fuze-maintenance.exe"), b"not-executed").unwrap();
        install::save(
            &root,
            &install::State {
                current: Some(install::Installed {
                    revision: "111111111111".into(),
                    sequence: 1,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        register_at(&name, &root, true).unwrap();
        register_at(&name, &root, true).unwrap();
        let key = open(&name).unwrap().unwrap();
        assert_eq!(read(&key, "DisplayVersion").unwrap(), "111111111111");
        assert!(read(&key, "UninstallString")
            .unwrap()
            .ends_with("\" --uninstall-local"));
        string(&key, "UnknownUserValue", "preserve").unwrap();
        assert_eq!(remove_at(&name, &root), Err("registration-unknown-value"));
        unsafe {
            RegDeleteValueW(key.0, wide("UnknownUserValue").as_ptr());
        }
        string(&key, "FuzeOwner", "other-owner").unwrap();
        assert_eq!(
            register_at(&name, &root, true),
            Err("registration-owner-conflict")
        );
        string(&key, "FuzeOwner", OWNER).unwrap();
        drop(key);
        remove_at(&name, &root).unwrap();
        assert!(open(&name).unwrap().is_none());
    }
}
