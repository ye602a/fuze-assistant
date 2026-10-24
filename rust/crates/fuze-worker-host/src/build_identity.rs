//! Package metadata is an allowlisted diagnostic hint, never a path or build attestation.
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::OnceLock};
pub const APP_VERSION: &str = include_str!("../../../app-version.txt");
/// Display the running package's metadata, never the online update channel.
pub fn release_label(directory: &Path) -> String {
    let path = directory.join("build.json");
    if !path.exists() {
        return format!("{} · 开发构建", APP_VERSION.trim());
    }
    crate::update_channel::bounded(&path, 16384)
        .ok()
        .and_then(|bytes| decode_release_label(&bytes))
        .unwrap_or_else(|| "版本 / 渠道未确认".into())
}
fn decode_release_label(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 16384 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    Revision::try_from(value["revision"].as_str()?.to_owned()).ok()?;
    let version = match value.get("version") {
        None => "0.7.0",
        Some(v) => v.as_str().filter(|v| semantic_version(v))?,
    };
    let channel = match value["channel"].as_str() {
        Some("rust-development-preview") => "测试渠道",
        _ => "渠道未确认",
    };
    Some(format!("{version} · {channel}"))
}
pub fn semantic_version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 6
                && (p.len() == 1 || !p.starts_with('0'))
                && p.bytes().all(|b| b.is_ascii_digit())
        })
}
pub fn package_version(directory: &Path) -> Option<String> {
    crate::managed_install::bundled_version(directory).ok()?;
    let value: serde_json::Value = serde_json::from_slice(
        &crate::update_channel::bounded(&directory.join("build.json"), 16384).ok()?,
    )
    .ok()?;
    // All published packages before the version field were v0.7.0.
    match value.get("version") {
        None => Some("0.7.0".into()),
        Some(v) => v
            .as_str()
            .filter(|v| semantic_version(v))
            .map(str::to_owned),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Revision(String);

impl TryFrom<String> for Revision {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if crate::update_channel::revision(&value) {
            Ok(Self(value))
        } else {
            Err("invalid-build-revision")
        }
    }
}

fn decode(bytes: &[u8]) -> Option<Revision> {
    if bytes.len() > 16384 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value["channel"] != "rust-development-preview" {
        return None;
    }
    Revision::try_from(value["revision"].as_str()?.to_owned()).ok()
}

fn for_executable(executable: &Path) -> Option<Revision> {
    let bytes =
        crate::update_channel::bounded(&executable.parent()?.join("build.json"), 16384).ok()?;
    decode(&bytes)
}

/// Read once from this executable's package, never the working directory or installed pointer.
/// A development binary or a missing/invalid manifest remains unknown.
pub fn current() -> Option<Revision> {
    static CURRENT: OnceLock<Option<Revision>> = OnceLock::new();
    CURRENT
        .get_or_init(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| for_executable(&p))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_label_uses_package_version_and_does_not_guess_unknown_channels() {
        let mut v = serde_json::json!({"revision":"123456789abc","version":"0.7.1","channel":"rust-development-preview"});
        assert_eq!(
            decode_release_label(v.to_string().as_bytes()).as_deref(),
            Some("0.7.1 · 测试渠道")
        );
        v["channel"] = serde_json::json!("unknown");
        assert_eq!(
            decode_release_label(v.to_string().as_bytes()).as_deref(),
            Some("0.7.1 · 渠道未确认")
        );
        v.as_object_mut().unwrap().remove("version");
        assert_eq!(
            decode_release_label(v.to_string().as_bytes()).as_deref(),
            Some("0.7.0 · 渠道未确认")
        );
        v["version"] = serde_json::json!("not-a-version");
        assert!(decode_release_label(v.to_string().as_bytes()).is_none());
        assert!(decode_release_label(b"{}").is_none());
    }

    #[test]
    fn only_bounded_package_revision_is_retained() {
        let valid = br#"{"revision":"1c2281940730","channel":"rust-development-preview","path":"private-do-not-export","token":"secret"}"#;
        assert_eq!(
            serde_json::to_string(&decode(valid)).unwrap(),
            "\"1c2281940730\""
        );
        for invalid in [
            "",
            "ABCDEF123456",
            "123456789ab",
            "123456789abcd",
            "private-token",
            "https://x/y",
        ] {
            assert!(Revision::try_from(invalid.to_owned()).is_err());
            assert!(serde_json::from_value::<Revision>(serde_json::json!(invalid)).is_err());
        }
        for invalid in [
            br#"{"revision":"1c2281940730"}"#.as_slice(),
            br#"{"revision":"1c2281940730","channel":"stable"}"#,
            br#"{"revision":123456789012,"channel":"rust-development-preview"}"#,
            b"not-json",
        ] {
            assert!(decode(invalid).is_none());
        }
        let mut oversized = valid.to_vec();
        oversized.resize(16385, b' ');
        assert!(decode(&oversized).is_none());
    }

    #[test]
    fn executable_directory_wins_and_missing_metadata_stays_unknown() {
        let root = std::env::temp_dir().join(format!("fuze-build-fixture-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let nested = root.join("portable");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(
            root.join("build.json"),
            br#"{"revision":"111111111111","channel":"rust-development-preview"}"#,
        )
        .unwrap();
        let executable = nested.join("fuze-ui-lab.exe");
        assert!(for_executable(&executable).is_none());
        std::fs::write(
            nested.join("build.json"),
            br#"{"revision":"222222222222","channel":"rust-development-preview"}"#,
        )
        .unwrap();
        assert_eq!(
            for_executable(&executable),
            Some(Revision::try_from("222222222222".to_owned()).unwrap())
        );
        std::fs::write(nested.join("build.json"), b"invalid-json").unwrap();
        assert!(for_executable(&executable).is_none());
        std::fs::write(nested.join("build.json"), vec![b' '; 16385]).unwrap();
        assert!(for_executable(&executable).is_none());
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("fuze-build-fixture-"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
