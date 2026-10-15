//! Small per-user preferences, outside immutable bundles. No credentials or game identity.
use crate::{controller::Config, managed_install::safe_dir, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub level_gate: bool,
    pub augment_overlay: bool,
    pub augment_side_panel: bool,
    pub schema: u32,
    pub backend: String,
    pub locale: String,
    pub champ_select: bool,
    pub champ_topmost: bool,
    pub close_to_tray: bool,
    pub private_updates: bool,
    pub icon_downloads: bool,
    pub pause_hotkey: bool,
    pub manual_directory: String,
    pub local_records: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            level_gate: true,
            augment_overlay: true,
            augment_side_panel: false,
            schema: 1,
            backend: "auto".into(),
            locale: "auto".into(),
            champ_select: true,
            champ_topmost: true,
            close_to_tray: true,
            private_updates: true,
            icon_downloads: true,
            pause_hotkey: false,
            manual_directory: String::new(),
            local_records: false,
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1
            || !["auto", "cpu", "dml"].contains(&self.backend.as_str())
            || !["auto", "zh-CN", "zh-TW", "en-US"].contains(&self.locale.as_str())
        {
            return Err("preferences-invalid");
        }
        if !self.manual_directory.is_empty()
            && (self.manual_directory.len() > 512
                || !Path::new(&self.manual_directory).is_absolute()
                || self.manual_directory.starts_with("\\\\")
                || self.manual_directory.chars().any(char::is_control))
        {
            return Err("preferences-directory");
        }
        Ok(())
    }
    pub fn apply(&self, config: &mut Config) -> Result<()> {
        self.validate()?;
        let locale = match self.locale.as_str() {
            "zh-CN" => Some(fuze_data::Locale::ZhCn),
            "zh-TW" => Some(fuze_data::Locale::ZhTw),
            "en-US" => Some(fuze_data::Locale::EnUs),
            _ => None,
        };
        if let Some(locale) = locale {
            fuze_data::prepare(&config.roots, locale, fuze_data::Scope::Champions)?;
        }
        config.level_gate = self.level_gate;
        config.augment_overlay = self.augment_overlay;
        config.icon_downloads = self.icon_downloads;
        config.backend.clone_from(&self.backend);
        config.locale = locale;
        Ok(())
    }
}
pub fn path() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("preferences-root")?)
            .join("FuzeRustPreview/preferences-v2.json"),
    )
}
pub fn read(path: &Path) -> Result<Preferences> {
    let legacy;
    let path = if !path.exists() && path.file_name().is_some_and(|n| n == "preferences-v2.json") {
        legacy = path.with_file_name("preferences.json");
        legacy.as_path()
    } else {
        path
    };
    if !path.exists() {
        return Ok(Preferences::default());
    }
    let value: Preferences = serde_json::from_slice(&crate::update_channel::bounded(path, 4096)?)
        .map_err(|_| "preferences-invalid")?;
    value.validate()?;
    Ok(value)
}
pub fn save(path: &Path, value: &Preferences) -> Result<()> {
    value.validate()?;
    if !value.manual_directory.is_empty()
        && !Path::new(&value.manual_directory)
            .join("LeagueClient.exe")
            .is_file()
    {
        return Err("preferences-directory-client-missing");
    }
    save_json(path, value)
}
/// Atomic writes shared by isolated preferences, geometry and reduced local records.
pub(crate) fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let parent = path.parent().ok_or("preferences-root")?;
    safe_dir(parent)?;
    let tmp = parent.join(format!(
        "preferences-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "preferences-clock")?
            .as_nanos()
    ));
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)
        .map_err(|_| "preferences-write")?;
    let result = (|| {
        f.write_all(&serde_json::to_vec(value).map_err(|_| "preferences-invalid")?)
            .map_err(|_| "preferences-write")?;
        f.sync_all().map_err(|_| "preferences-write")?;
        drop(f);
        let wide = |p: &Path| {
            p.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        if unsafe {
            MoveFileExW(
                wide(&tmp).as_ptr(),
                wide(path).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err("preferences-write");
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_preferences_receive_safe_defaults_and_reject_remote_fallback() {
        let mut p:Preferences=serde_json::from_str(r#"{"schema":1,"backend":"cpu","locale":"auto","champ_select":true,"champ_topmost":false,"close_to_tray":true}"#).unwrap();
        p.validate().unwrap();
        assert!(p.private_updates);
        assert!(!p.pause_hotkey);
        p.manual_directory = r"\\server\share".into();
        assert!(p.validate().is_err());
        p.manual_directory = "../private".into();
        assert!(p.validate().is_err());
    }
    #[test]
    fn preferences_roundtrip_reject_invalid_and_preserve_on_locked_commit() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir =
            std::env::temp_dir().join(format!("fuze-preferences-test-{}", std::process::id()));
        let path = dir.join("preferences.json");
        let value = Preferences::default();
        save(&path, &value).unwrap();
        assert_eq!(read(&path).unwrap(), value);
        let mut changed = value.clone();
        changed.backend = "cpu".into();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(save(&path, &changed).is_err());
        drop(lock);
        assert_eq!(read(&path).unwrap(), value);
        changed.locale = "../private".into();
        assert!(save(&path, &changed).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
