//! Explicit user-selected, read-only Electron settings import. Never copy unknown fields.
use crate::{preferences::Preferences, Result};
use serde_json::Value;
pub fn read(path: &std::path::Path, current: &Preferences) -> Result<Preferences> {
    let bytes = crate::update_channel::bounded(path, 256 * 1024)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "legacy-settings-invalid")?;
    convert(&value, current)
}
fn convert(value: &Value, current: &Preferences) -> Result<Preferences> {
    if !value.is_object() {
        return Err("legacy-settings-invalid");
    }
    let field = |group: &str, key: &str| -> Result<Option<&Value>> {
        let flat = value.get(format!("{group}.{key}"));
        let nested = value.get(group).and_then(|v| v.get(key));
        if flat.is_some() && nested.is_some() && flat != nested {
            return Err("legacy-settings-conflict");
        }
        Ok(flat.or(nested))
    };
    let mut next = current.clone();
    let mut count = 0;
    if let Some(v) = value.get("autoScreenshotGameflowControl") {
        next.automatic = v.as_bool().ok_or("legacy-settings-type")?;
        count += 1;
    }
    if let Some(v) = field("app", "locale")? {
        next.locale = v
            .as_str()
            .filter(|s| ["zh-CN", "zh-TW", "en-US"].contains(s))
            .ok_or("legacy-settings-locale")?
            .into();
        count += 1;
    }
    if let Some(v) = field("ocr", "gpuEnabled")? {
        next.backend = if v.as_bool().ok_or("legacy-settings-type")? {
            "auto"
        } else {
            "cpu"
        }
        .into();
        count += 1;
    }
    for (group, key, target) in [
        ("ocr", "levelGateEnabled", &mut next.level_gate),
        ("augments", "showTopOverlay", &mut next.augment_overlay),
        ("augments", "showSidePanel", &mut next.augment_side_panel),
    ] {
        if let Some(v) = field(group, key)? {
            *target = v.as_bool().ok_or("legacy-settings-type")?;
            count += 1;
        }
    }
    for (key, target) in [
        ("showDetails", &mut next.champ_select),
        ("pinDuringChampSelect", &mut next.champ_topmost),
    ] {
        if let Some(v) = field("championInsight", key)? {
            *target = v.as_bool().ok_or("legacy-settings-type")?;
            count += 1;
        }
    }
    if let Some(v) = value.get("lolPath") {
        next.manual_directory = v.as_str().ok_or("legacy-settings-type")?.to_owned();
        count += 1;
    }
    if count == 0 {
        return Err("legacy-settings-no-supported-fields");
    }
    next.validate()?;
    Ok(next)
}
/// Open only after an explicit settings button click. No directory scanning or automatic import.
pub unsafe fn choose(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<std::path::PathBuf> {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut file = [0u16; 32768];
    let filter: Vec<u16> = "JSON\0*.json\0\0".encode_utf16().collect();
    let mut dialog: OPENFILENAMEW = std::mem::zeroed();
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = hwnd;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_DONTADDTORECENT;
    if GetOpenFileNameW(&mut dialog) == 0 {
        return None;
    }
    let n = file.iter().position(|c| *c == 0)?;
    Some(std::path::PathBuf::from(String::from_utf16_lossy(
        &file[..n],
    )))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_only_allowlist_and_rejects_conflicting_or_bad_types() {
        let current = Preferences::default();
        let v = serde_json::json!({"app":{"locale":"en-US"},"ocr.gpuEnabled":false,"championInsight":{"showDetails":false},"token":"fixture-secret","puuid":"fixture-identity","history":[1,2]});
        let p = convert(&v, &current).unwrap();
        assert_eq!(p.locale, "en-US");
        assert_eq!(p.backend, "cpu");
        assert!(!p.champ_select);
        assert!(
            !convert(
                &serde_json::json!({"autoScreenshotGameflowControl":false}),
                &current
            )
            .unwrap()
            .automatic
        );
        let b = serde_json::to_string(&p).unwrap();
        assert!(!b.contains("fixture"));
        assert!(p.private_updates);
        assert!(convert(
            &serde_json::json!({"app.locale":"en-US","app":{"locale":"zh-CN"}}),
            &current
        )
        .is_err());
        assert!(convert(&serde_json::json!({"ocr.gpuEnabled":"false"}), &current).is_err());
        assert!(convert(&serde_json::json!({"token":"fixture"}), &current).is_err());
        assert_eq!(current, Preferences::default());
    }
}
