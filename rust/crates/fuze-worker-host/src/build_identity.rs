//! Package metadata is an allowlisted diagnostic hint, never a path or build attestation.
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::OnceLock};

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
