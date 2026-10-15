#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
fn run() -> Result<(), &'static str> {
    use fuze_worker_host::{
        managed_install as install,
        update_channel::{self, Session},
        update_credential,
    };
    use std::{fs, io::Write};
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 4 && args[0] == "--verify-package" {
        use std::io::Read;
        let manifest: update_channel::Manifest = serde_json::from_slice(&update_channel::bounded(
            std::path::Path::new(&args[1]),
            16384,
        )?)
        .map_err(|_| "update-manifest-format")?;
        manifest.validate()?;
        let file = fs::File::open(&args[2]).map_err(|_| "update-read")?;
        if file.metadata().map_err(|_| "update-read")?.len() != manifest.size {
            return Err("update-size");
        }
        let mut b = Vec::new();
        file.take(512 * 1024 * 1024 + 1)
            .read_to_end(&mut b)
            .map_err(|_| "update-read")?;
        if fuze_worker_host::update_io::digest(&b, true)?
            != update_channel::decode_hex(&manifest.sha512, 64)?
        {
            return Err("update-final-hash");
        }
        drop(b);
        let target = std::path::Path::new(&args[3]);
        install::unpack(std::path::Path::new(&args[2]), target, &manifest.revision)?;
        install::check_bundle(target)?;
        println!("{{\"package_verified\":true,\"native_extracted\":true,\"worker_reaped\":true,\"game_access\":false}}");
        return Ok(());
    }
    let action = args.first().map(String::as_str).unwrap_or("--launch");
    if args.len() > 1 {
        return Err("update-arguments");
    }
    if action == "--credentials" {
        return update_credential::prompt();
    }
    if action == "--forget-credentials" {
        update_credential::remove()?;
        message("私有频道令牌已删除。公开频道无需令牌。", false);
        return Ok(());
    }
    let bundled = std::env::current_exe()
        .map_err(|_| "update-location")?
        .parent()
        .ok_or("update-location")?
        .to_path_buf();
    let root = install::root()?;
    if action == "--launch" {
        return install::launch(&root, &bundled);
    }
    if action == "--rollback" {
        if install::desktop_running() {
            return Err("install-close-preview-first");
        }
        let _lock = install::lock(&root)?;
        install::rollback(&root)?;
        message("已切回上一版，下次启动生效。", false);
        return Ok(());
    }
    if !["--update", "--update-private"].contains(&action) {
        return Err("update-arguments");
    }
    if !message("检查 Rust 开发频道更新？公开频道无需令牌。下载和安装均由你确认；不会操作游戏或 Electron 稳定版。",true){return Ok(());}
    let _lock = install::lock(&root)?;
    let session = if action == "--update-private" {
        match Session::private() {
            Ok(session) => session,
            Err("credential-unavailable") => {
                update_credential::prompt()?;
                Session::private()?
            }
            Err(error) => return Err(error),
        }
    } else {
        Session::public()
    };
    fuze_worker_host::update_progress::STAGE.store(1, std::sync::atomic::Ordering::Release);
    let release = session.check(&fuze_worker_host::update_progress::cancelled)?;
    let state = install::read(&root)?;
    if state.pending {
        return Err("install-validation-pending");
    }
    if state
        .current
        .as_ref()
        .is_some_and(|v| v.sequence >= release.manifest.sequence)
        || state.rejected.as_ref() == Some(&release.manifest.revision)
    {
        message(
            "当前已是同版/更新版本，或此版本曾启动失败。不会重复安装。",
            false,
        );
        return Ok(());
    }
    if !message(&format!("发现 Rust 版本 {}，完整包约 {} MB。下载并校验？有可用基线时优先差分，失败最多回退一次全量。",release.manifest.revision,release.manifest.size/1024/1024),true){return Ok(());}
    let stage = root.join(format!(
        "pending-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "install-clock")?
            .as_nanos()
    ));
    if stage.exists() {
        return Err("update-stage-exists");
    }
    install::safe_dir(&stage)?;
    let result = (|| {
        let destination = stage.join("package.fuz");
        let baseline = state
            .current
            .as_ref()
            .map(|v| root.join("versions").join(&v.revision));
        let baseline_archive = baseline.as_ref().map(|p| p.join("package.fuz"));
        let baseline_map = baseline.as_ref().and_then(|p| {
            update_channel::bounded(&p.join("package.blockmap"), 8 * 1024 * 1024).ok()
        });
        let old = baseline_archive
            .as_deref()
            .zip(baseline_map.as_deref())
            .filter(|(p, _)| p.is_file());
        // Blockmaps are optional optimization; failed/missing maps cause one full download.
        let map = session
            .bytes(
                &release.map,
                8 * 1024 * 1024,
                &fuze_worker_host::update_progress::cancelled,
            )
            .unwrap_or_default();
        fuze_worker_host::update_progress::STAGE.store(2, std::sync::atomic::Ordering::Release);
        let transfer = update_channel::fetch_with(
            &release.manifest,
            &destination,
            old,
            &map,
            &|offset, length| {
                let mut b = Vec::new();
                session.download(
                    &release.archive,
                    length as usize,
                    Some((offset, offset + length - 1, release.manifest.size)),
                    &mut b,
                    &fuze_worker_host::update_progress::cancelled,
                )?;
                Ok(b)
            },
            &|out| {
                session.download(
                    &release.archive,
                    release.manifest.size as usize,
                    None,
                    out,
                    &fuze_worker_host::update_progress::cancelled,
                )
            },
            &fuze_worker_host::update_progress::cancelled,
        )?;
        fuze_worker_host::update_progress::STAGE.store(3, std::sync::atomic::Ordering::Release);
        let extracted = stage.join("version");
        install::unpack(&destination, &extracted, &release.manifest.revision)?;
        install::check_bundle(&extracted)?;
        if install::desktop_running() {
            return Err("install-close-preview-first");
        }
        if !message(&format!("下载与离线 OCR 自检通过（{}，下载 {} MB）。是否安装到独立 Rust 目录？保留上一版，下一次启动前再次自检，失败自动回退。",transfer.mode,transfer.downloaded/1024/1024),true){return Ok(());}
        let versions = root.join("versions");
        install::safe_dir(&versions)?;
        let target = versions.join(&release.manifest.revision);
        if target.exists() {
            if [state.current.as_ref(), state.previous.as_ref()]
                .into_iter()
                .flatten()
                .any(|v| v.revision == release.manifest.revision)
            {
                return Err("install-version-in-use");
            }
            install::safe_dir(&target)?;
            fs::rename(&target, stage.join("abandoned-version"))
                .map_err(|_| "install-orphan-recovery")?;
        }
        fs::rename(destination, extracted.join("package.fuz")).map_err(|_| "install-cache")?;
        if !map.is_empty() {
            let mut f = fs::File::create(extracted.join("package.blockmap"))
                .map_err(|_| "install-cache")?;
            f.write_all(&map).map_err(|_| "install-cache")?;
        }
        fs::rename(&extracted, &target).map_err(|_| "install-version")?;
        if fuze_worker_host::update_progress::cancelled() {
            return Err("update-cancelled");
        }
        fuze_worker_host::update_progress::STAGE.store(4, std::sync::atomic::Ordering::Release);
        install::bootstrap(&root, &bundled)?;
        if fuze_worker_host::update_progress::cancelled() {
            return Err("update-cancelled");
        }
        if install::desktop_running() {
            return Err("install-close-preview-first");
        }
        install::activate(&root, &release.manifest, &|_| Ok(()))?;
        message("Rust 更新已准备好。下次使用 Start-preview.cmd 启动新版；验证失败自动回到上一版。没有重启任何程序。",false);
        Ok(())
    })();
    // Delete only the exact freshly created staging directory under this isolated root.
    if fs::canonicalize(&stage).ok().as_ref() == Some(&stage)
        && stage.parent() == Some(root.as_path())
    {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}
#[cfg(windows)]
fn message(text: &str, question: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    unsafe {
        MessageBoxW(
            fuze_worker_host::update_progress::OWNER.load(std::sync::atomic::Ordering::Acquire)
                as _,
            wide(text).as_ptr(),
            wide("符择 · Rust 更新").as_ptr(),
            if question {
                MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2
            } else {
                MB_OK | MB_ICONINFORMATION
            },
        ) == IDYES
    }
}
fn main() {
    #[cfg(windows)]
    if let Err(error) = if matches!(
        std::env::args().nth(1).as_deref(),
        Some("--update") | Some("--update-private")
    ) {
        fuze_worker_host::update_progress::run(run)
    } else {
        run()
    } {
        if std::env::args().nth(1).as_deref() == Some("--verify-package") {
            eprintln!("{error}");
            std::process::exit(1);
        }
        message(&format!("操作未完成：{error}\n原有版本与 Electron 稳定版保持不变。私有频道请使用 Private-channel.cmd 管理令牌；公开仓库无需令牌。"),false);
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    std::process::exit(1);
}
