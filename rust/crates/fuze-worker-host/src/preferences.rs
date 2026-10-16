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
    pub automatic: bool,
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
            automatic: true,
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
    pub fn recognition_changed(&self, other: &Self) -> bool {
        self.level_gate != other.level_gate
            || self.augment_overlay != other.augment_overlay
            || self.backend != other.backend
            || self.locale != other.locale
            || self.icon_downloads != other.icon_downloads
            || self.manual_directory != other.manual_directory
            || self.local_records != other.local_records
    }
    pub fn starts_automatically(&self, fixture: bool, valid: bool) -> bool {
        self.automatic && !fixture && valid
    }
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
        if let Some(locale) = self.selected_locale() {
            fuze_data::prepare(&config.roots, locale, fuze_data::Scope::Champions)?;
        }
        self.apply_values(config);
        Ok(())
    }
    fn selected_locale(&self) -> Option<fuze_data::Locale> {
        match self.locale.as_str() {
            "zh-CN" => Some(fuze_data::Locale::ZhCn),
            "zh-TW" => Some(fuze_data::Locale::ZhTw),
            "en-US" => Some(fuze_data::Locale::EnUs),
            _ => None,
        }
    }
    fn apply_values(&self, config: &mut Config) {
        config.level_gate = self.level_gate;
        config.augment_overlay = self.augment_overlay;
        config.icon_downloads = self.icon_downloads;
        config.backend.clone_from(&self.backend);
        config.locale = self.selected_locale();
    }
}
/// Prepare first, persist second, publish last. No settings/catalogue are changed on failure.
/// Full locale validation only runs for an explicit language change, never per frame or
/// when toggling unrelated UI preferences. Images remain optional, locally cached assets.
pub(crate) fn commit_change(
    current: &mut Preferences,
    config: &mut Config,
    candidate: Preferences,
    path: Option<&Path>,
) -> Result<bool> {
    candidate.validate()?;
    let restart = current.recognition_changed(&candidate);
    let mut next = config.clone();
    if candidate.locale != current.locale {
        if let Some(locale) = candidate.selected_locale() {
            let complete = fuze_data::prepare(&next.roots, locale, fuze_data::Scope::Full)?;
            let shards = complete
                .document("champion-shards/index.json")
                .and_then(|d| d.get("shards"))
                .and_then(serde_json::Value::as_array)
                .ok_or("preferences-locale-incomplete")?;
            if shards.len() > 512 {
                return Err("preferences-locale-incomplete");
            }
            for shard in shards {
                let path = shard
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("preferences-locale-incomplete")?;
                if !path.starts_with("champion-shards/")
                    || path == "champion-shards/index.json"
                    || complete.document(path).is_none()
                {
                    return Err("preferences-locale-incomplete");
                }
            }
            let foreground = fuze_data::prepare(&next.roots, locale, fuze_data::Scope::Ocr)?;
            // A newer partial cache must not supply OCR while details fall back to an older bundle.
            if complete.identity() != foreground.identity()
                || complete.source() != foreground.source()
            {
                return Err("preferences-locale-incomplete");
            }
        }
        candidate.apply_values(&mut next);
    } else if restart {
        candidate.apply(&mut next)?;
    }
    if let Some(path) = path {
        save(path, &candidate)?;
    }
    *config = next;
    *current = candidate;
    Ok(restart)
}
pub fn path() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("preferences-root")?)
            .join("FuzeRustPreview/preferences-v3.json"),
    )
}
pub fn read(path: &Path) -> Result<Preferences> {
    let legacy;
    let path = if !path.exists()
        && path
            .file_name()
            .is_some_and(|n| n == "preferences-v2.json" || n == "preferences-v3.json")
    {
        let v2 = path.with_file_name("preferences-v2.json");
        legacy = if v2.exists() {
            v2
        } else {
            path.with_file_name("preferences.json")
        };
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
        && !fuze_lcu::install::is_client_directory(Path::new(&value.manual_directory))
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
#[path = "locale_activation_tests.rs"]
pub(crate) mod locale_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_preferences_receive_safe_defaults_and_reject_remote_fallback() {
        let mut p:Preferences=serde_json::from_str(r#"{"schema":1,"backend":"cpu","locale":"auto","champ_select":true,"champ_topmost":false,"close_to_tray":true}"#).unwrap();
        p.validate().unwrap();
        assert!(p.starts_automatically(false, true));
        assert!(!p.starts_automatically(true, true));
        assert!(!p.starts_automatically(false, false));
        p.automatic = false;
        assert!(!p.starts_automatically(false, true));
        let mut ui_only = p.clone();
        ui_only.champ_topmost = !ui_only.champ_topmost;
        ui_only.close_to_tray = !ui_only.close_to_tray;
        assert!(!p.recognition_changed(&ui_only));
        ui_only.backend = "dml".into();
        assert!(p.recognition_changed(&ui_only));
        assert!(p.private_updates);
        assert!(!p.pause_hotkey);
        p.manual_directory = r"\\server\share".into();
        assert!(p.validate().is_err());
        p.manual_directory = "../private".into();
        assert!(p.validate().is_err());
    }
    #[test]
    fn manual_directory_saves_root_and_nested_layouts_without_process_access() {
        let dir = std::env::temp_dir().join(format!(
            "fuze-directory-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("client/LeagueClient")).unwrap();
        fs::write(dir.join("client/LeagueClient/LeagueClient.exe"), b"fixture").unwrap();
        let path = dir.join("preferences.json");
        for root in [dir.join("client"), dir.join("client/LeagueClient")] {
            let mut value = Preferences::default();
            value.manual_directory = root.to_string_lossy().into_owned();
            save(&path, &value).unwrap();
            assert_eq!(read(&path).unwrap(), value);
        }
        let previous = read(&path).unwrap();
        let mut invalid = previous.clone();
        invalid.manual_directory = dir.join("missing").to_string_lossy().into_owned();
        assert_eq!(
            save(&path, &invalid),
            Err("preferences-directory-client-missing")
        );
        assert_eq!(read(&path).unwrap(), previous);
        // Only the files created by this test.
        fs::remove_file(path).unwrap();
        fs::remove_file(dir.join("client/LeagueClient/LeagueClient.exe")).unwrap();
        fs::remove_dir(dir.join("client/LeagueClient")).unwrap();
        fs::remove_dir(dir.join("client")).unwrap();
        fs::remove_dir(dir).unwrap();
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
