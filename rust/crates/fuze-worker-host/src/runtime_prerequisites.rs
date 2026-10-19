//! Read-only preflight before executing a managed bundle. No DLL loading or installer effects.
use crate::{update_channel, update_io, Result};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::{Storage::FileSystem::*, System::SystemInformation::GetSystemDirectoryW};

const DLLS: [&str; 7] = [
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "msvcp140.dll",
    "msvcp140_1.dll",
    "msvcp140_2.dll",
    "msvcp140_atomic_wait.dll",
    "concrt140.dll",
];
type Version = [u32; 4];
fn version(s: &str, major: u32) -> Option<Version> {
    let parts = s
        .split('.')
        .map(|p| {
            (!p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
                .then(|| p.parse::<u32>().ok())
                .flatten()
        })
        .collect::<Option<Vec<_>>>()?;
    let parts: Version = parts.try_into().ok()?;
    (parts[0] == major).then_some(parts)
}
fn sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
struct Contract {
    minimum: Version,
    dlls: Vec<String>,
    binaries: Vec<(String, String)>,
}
fn contract(build: &Value) -> Result<Option<Contract>> {
    // Missing means unknown legacy evidence, not today's minimum. Preserve legacy health checks.
    let Some(c) = build.get("vcRuntime") else {
        return Ok(None);
    };
    let text = |key: &str| c[key].as_str().ok_or("runtime-contract-invalid");
    let minimum = version(text("minimumVersion")?, 14).ok_or("runtime-contract-invalid")?;
    let linker = version(text("linkerVersion")?, 14).ok_or("runtime-contract-invalid")?;
    let mut compiler = version(text("compilerVersion")?, 19).ok_or("runtime-contract-invalid")?;
    compiler[0] = 14;
    let mut names = vec![
        "fuze-rust-host.exe",
        "fuze-ocr-worker.exe",
        "fuze-update.exe",
        "onnxruntime.dll",
        "DirectML.dll",
        "dxcompiler.dll",
        "dxil.dll",
    ];
    match build["uiFrontend"].as_str() {
        Some("slint") => names.push("fuze-ui-lab.exe"),
        Some("win32") => (),
        _ => return Err("runtime-contract-invalid"),
    }
    let dlls = c["requiredDlls"]
        .as_array()
        .ok_or("runtime-contract-invalid")?;
    let binaries = c["binaries"]
        .as_object()
        .ok_or("runtime-contract-invalid")?;
    let revision = build["revision"].as_str().unwrap_or("");
    let redist = &c["redistributable"];
    let url = redist["url"].as_str().unwrap_or("");
    let microsoft = url
        .strip_prefix("https://download.visualstudio.microsoft.com/download/pr/")
        .and_then(|s| s.strip_suffix("/VC_redist.x64.exe"))
        .is_some_and(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/-".contains(&b))
        });
    if c["schema"] != 1
        || c["architecture"] != "x64"
        || c["basis"] != "supported-baseline-not-lowest-abi-version"
        || !(12..=40).contains(&revision.len())
        || !revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || minimum < linker
        || minimum < compiler
        || !sha(text("policySha256")?)
        || redist["version"] != c["minimumVersion"]
        || !sha(redist["sha256"].as_str().unwrap_or(""))
        || !microsoft
        || dlls.is_empty()
        || dlls.len() > DLLS.len()
        || binaries.len() != names.len()
    {
        return Err("runtime-contract-invalid");
    }
    let mut required = Vec::new();
    for name in dlls {
        let name = name.as_str().ok_or("runtime-contract-invalid")?;
        if !DLLS.contains(&name) || required.iter().any(|n| n == name) {
            return Err("runtime-contract-invalid");
        }
        required.push(name.to_owned());
    }
    let mut hashes = Vec::new();
    for name in names {
        let hash = binaries
            .get(name)
            .and_then(Value::as_str)
            .filter(|s| sha(s))
            .ok_or("runtime-contract-invalid")?;
        hashes.push((name.into(), hash.into()));
    }
    Ok(Some(Contract {
        minimum,
        dlls: required,
        binaries: hashes,
    }))
}
fn x64(path: &Path) -> bool {
    (|| -> Option<bool> {
        let mut f = fs::File::open(path).ok()?;
        let mut dos = [0; 64];
        f.read_exact(&mut dos).ok()?;
        if &dos[..2] != b"MZ" {
            return Some(false);
        }
        let offset = u32::from_le_bytes(dos[60..64].try_into().ok()?) as u64;
        if offset > f.metadata().ok()?.len().saturating_sub(26) {
            return Some(false);
        }
        f.seek(SeekFrom::Start(offset)).ok()?;
        let mut pe = [0; 26];
        f.read_exact(&mut pe).ok()?;
        Some(&pe[..4] == b"PE\0\0" && pe[4..6] == [0x64, 0x86] && pe[24..26] == [0x0b, 2])
    })()
    .unwrap_or(false)
}
fn system_directory() -> Result<PathBuf> {
    let mut path = [0u16; 32768];
    let len = unsafe { GetSystemDirectoryW(path.as_mut_ptr(), path.len() as u32) } as usize;
    if len == 0 || len >= path.len() {
        return Err("runtime-inspection-failed");
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&path[..len])))
}
fn file_version(path: &Path) -> Option<Version> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut());
        if size == 0 || size > 1024 * 1024 {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let mut info = std::ptr::null_mut();
        let mut len = 0;
        if VerQueryValueW(
            buffer.as_ptr().cast(),
            [92u16, 0].as_ptr(),
            &mut info,
            &mut len,
        ) == 0
            || info.is_null()
            || len < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
        {
            return None;
        }
        let fixed = std::ptr::read_unaligned(info.cast::<VS_FIXEDFILEINFO>());
        if fixed.dwSignature != 0xfeef04bd {
            return None;
        }
        Some([
            fixed.dwFileVersionMS >> 16,
            fixed.dwFileVersionMS & 0xffff,
            fixed.dwFileVersionLS >> 16,
            fixed.dwFileVersionLS & 0xffff,
        ])
    }
}
fn inspect(path: &Path) -> Option<Version> {
    if !x64(path) {
        return None;
    }
    file_version(path)
}
fn check_with(directory: &Path, inventory: &dyn Fn(&str) -> Option<Version>) -> Result<bool> {
    let meta = fs::symlink_metadata(directory.join("build.json"))
        .map_err(|_| "runtime-contract-invalid")?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return Err("runtime-contract-invalid");
    }
    let bytes = update_channel::bounded(&directory.join("build.json"), 16384)
        .map_err(|_| "runtime-contract-invalid")?;
    let build = serde_json::from_slice(&bytes).map_err(|_| "runtime-contract-invalid")?;
    let Some(contract) = contract(&build)? else {
        return Ok(false);
    };
    // Reject changed native bytes before trusting their recorded dependency list.
    for (name, hash) in contract.binaries {
        let path = directory.join(name);
        let meta = fs::symlink_metadata(&path).map_err(|_| "runtime-artifact-changed")?;
        if !meta.is_file()
            || meta.file_attributes() & 0x400 != 0
            || meta.len() > 128 * 1024 * 1024
            || !x64(&path)
        {
            return Err("runtime-artifact-changed");
        }
        let bytes = update_channel::bounded(&path, 128 * 1024 * 1024)
            .map_err(|_| "runtime-artifact-changed")?;
        if update_io::hex(&update_io::digest(&bytes, false)?) != hash {
            return Err("runtime-artifact-changed");
        }
    }
    for name in contract.dlls {
        if inventory(&name).is_none_or(|v| v[0] != 14 || v < contract.minimum) {
            return Err("runtime-required");
        }
    }
    Ok(true)
}
/// `false` preserves legacy compatibility and means unknown, never dependency acceptance.
pub fn check(directory: &Path) -> Result<bool> {
    let system = system_directory()?;
    check_with(directory, &|name| inspect(&system.join(name)))
}
pub fn guidance(error: &str) -> Option<&'static str> {
    match error {
        "runtime-required" | "runtime-inspection-failed" => Some("未能确认本版本所需的 Visual C++ x64 运行库。请从本版本原始 ZIP 运行 Install-local.cmd 或 Start-this-build.cmd 检查并按提示处理依赖。没有自动安装运行库。"),
        "runtime-contract-invalid" | "runtime-artifact-changed" => Some("本版本的构建记录或程序文件不完整，请重新取得原始 ZIP。没有执行未通过检查的程序。"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Value) {
        let root = std::env::temp_dir().join(format!(
            "fuze-runtime-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let mut pe = vec![0u8; 256];
        pe[..2].copy_from_slice(b"MZ");
        pe[60..64].copy_from_slice(&128u32.to_le_bytes());
        pe[128..134].copy_from_slice(&[b'P', b'E', 0, 0, 0x64, 0x86]);
        pe[152..154].copy_from_slice(&[0x0b, 2]);
        let mut binaries = serde_json::Map::new();
        for name in [
            "fuze-rust-host.exe",
            "fuze-ocr-worker.exe",
            "fuze-update.exe",
            "fuze-ui-lab.exe",
            "onnxruntime.dll",
            "DirectML.dll",
            "dxcompiler.dll",
            "dxil.dll",
        ] {
            fs::write(root.join(name), &pe).unwrap();
            binaries.insert(
                name.into(),
                Value::String(update_io::hex(&update_io::digest(&pe, false).unwrap())),
            );
        }
        let policy: Value =
            serde_json::from_str(include_str!("../../../vc-runtime-policy.json")).unwrap();
        let build = serde_json::json!({"revision":"123456789abc","uiFrontend":"slint","vcRuntime":{
            "schema":1,"architecture":"x64","minimumVersion":"14.51.36247.0",
            "basis":"supported-baseline-not-lowest-abi-version","compilerVersion":"19.44.35229.0",
            "linkerVersion":"14.44.35229.0","policySha256":"a".repeat(64),
            "requiredDlls":["vcruntime140.dll","msvcp140.dll"],"binaries":binaries,"redistributable":policy["redistributable"]}});
        (root, build)
    }
    fn write(root: &Path, build: &Value) {
        fs::write(root.join("build.json"), serde_json::to_vec(build).unwrap()).unwrap()
    }
    #[test]
    fn unknown_old_reports_do_not_invent_a_minimum_or_inspect_system_files() {
        let (root, _) = fixture();
        write(&root, &serde_json::json!({"revision":"123456789abc"}));
        assert_eq!(
            check_with(&root, &|_| panic!("legacy must stay unknown")),
            Ok(false)
        );
    }
    #[test]
    fn complete_dependency_inventory_and_native_hashes_are_required() {
        let (root, build) = fixture();
        write(&root, &build);
        assert_eq!(check_with(&root, &|_| Some([14, 51, 36247, 0])), Ok(true));
        assert_eq!(check_with(&root, &|_| Some([14, 52, 0, 0])), Ok(true));
        for actual in [
            None,
            Some([14, 50, 99999, 0]),
            Some([14, 9, 99999, 0]),
            Some([13, 99, 0, 0]),
        ] {
            assert_eq!(check_with(&root, &|_| actual), Err("runtime-required"));
        }
        assert_eq!(
            check_with(&root, &|n| (n == "vcruntime140.dll")
                .then_some([14, 51, 36247, 0])),
            Err("runtime-required")
        );
        fs::write(root.join("fuze-ui-lab.exe"), b"changed, never executable").unwrap();
        assert_eq!(
            check_with(&root, &|_| panic!("changed bytes")),
            Err("runtime-artifact-changed")
        );
        // Production entry must fail here rather than executing any of the inert fixtures.
        assert_eq!(
            crate::managed_install::check_bundle(&root),
            Err("runtime-artifact-changed")
        );
    }
    #[test]
    fn malformed_or_weakened_records_do_not_fall_back_to_legacy() {
        let (_, build) = fixture();
        for (key, value) in [
            ("schema", serde_json::json!(2)),
            ("minimumVersion", serde_json::json!("14.1.0.0")),
            ("requiredDlls", serde_json::json!(["../private.dll"])),
            (
                "requiredDlls",
                serde_json::json!(["vcruntime140.dll", "vcruntime140.dll"]),
            ),
            ("requiredDlls", serde_json::json!([])),
            ("binaries", serde_json::json!({})),
        ] {
            let mut invalid = build.clone();
            invalid["vcRuntime"][key] = value;
            assert!(matches!(
                contract(&invalid),
                Err("runtime-contract-invalid")
            ));
        }
        let mut invalid = build;
        invalid["vcRuntime"] = Value::Null;
        assert!(contract(&invalid).is_err());
    }
    #[test]
    fn reads_only_fixed_system_dll_metadata_without_loading_a_library() {
        let system = system_directory().unwrap();
        let kernel = system.join("kernel32.dll");
        assert!(x64(&kernel));
        assert!(file_version(&kernel).is_some());
        assert_eq!(inspect(&system.join("fuze-nonexistent-fixture.dll")), None);
    }
    #[test]
    fn missing_runtime_preserves_pointer_and_pending_failure_starts_only_previous_fixture() {
        use crate::managed_install::{self as install, Installed, State};
        let (root, build) = fixture();
        write(&root, &build);
        let initial = State {
            current: Some(Installed {
                revision: "111111111111".into(),
                sequence: 1,
            }),
            ..Default::default()
        };
        install::save(&root, &initial).unwrap();
        let manifest = update_channel::Manifest {
            schema: 1,
            channel: "rust-preview".into(),
            revision: "222222222222".into(),
            sequence: 2,
            archive: "Fuze-Rust-222222222222.fuz".into(),
            size: 1,
            sha512: "00".repeat(64),
            blockmap_sha256: "00".repeat(32),
        };
        let missing = |_: &Path| check_with(&root, &|_| None).map(|_| ());
        let pointer = fs::read(root.join("current.json")).unwrap();
        assert_eq!(
            install::activate(&root, &manifest, &missing),
            Err("runtime-required")
        );
        assert_eq!(fs::read(root.join("current.json")).unwrap(), pointer);
        // Simulate an already pending candidate whose runtime was removed before next launch.
        install::activate(&root, &manifest, &|_| Ok(())).unwrap();
        let starts = std::cell::Cell::new(0);
        install::launch_with(&root, &root, &missing, &|path| {
            assert_eq!(path.file_name().unwrap(), "111111111111");
            starts.set(starts.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(starts.get(), 1);
        assert_eq!(install::read(&root).unwrap().current, initial.current);
        install::save(&root, &initial).unwrap();
        assert_eq!(
            install::launch_with(&root, &root, &|_| Ok(()), &missing),
            Err("runtime-required")
        );
        assert_eq!(install::read(&root).unwrap(), initial);
    }
}
