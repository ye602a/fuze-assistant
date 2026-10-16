//! Creates only fresh synthetic files below the explicit review output directory.
use std::{
    fs::{self, FileTimes, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
pub fn fixture(output: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = output.join(format!("maintenance-fixture-{}", std::process::id()));
    fs::create_dir(&root)?;
    let root = fs::canonicalize(root)?;
    for version in ["1.0.0", "1.0.1", "1.0.2"] {
        let dir = root.join("client-data/versions").join(version);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("augments.json"), b"{}")?;
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(
                &serde_json::json!({"dataVersion":version,"locale":"zh-CN","files":{"augments.json":{"bytes":2}}}),
            )?,
        )?;
        for path in [dir.join("augments.json"), dir.join("manifest.json"), dir] {
            let f = OpenOptions::new()
                .access_mode(0x100)
                .custom_flags(0x02000000)
                .open(path)?;
            f.set_times(
                FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(8 * 86400)),
            )?;
        }
    }
    fs::write(
        root.join("client-data/current.json"),
        br#"{"dataVersion":"1.0.2"}"#,
    )?;
    fs::write(
        root.join("observations.json"),
        b"synthetic-preserved-record",
    )?;
    Ok(root)
}
pub fn preserved(root: &Path) -> bool {
    !root.join("client-data/versions/1.0.0").exists()
        && root
            .join("client-data/versions/1.0.1/augments.json")
            .is_file()
        && root
            .join("client-data/versions/1.0.2/augments.json")
            .is_file()
        && fs::read(root.join("observations.json")).ok().as_deref()
            == Some(b"synthetic-preserved-record")
}
