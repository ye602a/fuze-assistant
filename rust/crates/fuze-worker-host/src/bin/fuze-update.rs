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
    if action == "--uninstall-local" {
        // Run outside the managed tree so Windows can release and delete installed binaries.
        use std::os::windows::process::CommandExt;
        let temporary = std::env::temp_dir().join(format!(
            "fuze-uninstall-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "install-clock")?
                .as_nanos()
        ));
        fs::create_dir(&temporary).map_err(|_| "uninstall-helper")?;
        install::safe_dir(&temporary)?;
        let helper = temporary.join("fuze-update.exe");
        fs::copy(
            std::env::current_exe().map_err(|_| "update-location")?,
            &helper,
        )
        .map_err(|_| "uninstall-helper")?;
        std::process::Command::new(helper)
            .arg("--uninstall-worker")
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| "uninstall-helper")?;
        return Ok(());
    }
    if action == "--uninstall-worker" {
        if fs::canonicalize(&bundled)
            .map_err(|_| "uninstall-helper-location")?
            .starts_with(&root)
        {
            return Err("uninstall-helper-location");
        }
        if !message("卸载独立 Rust 预览版和开始菜单快捷方式？设置、个人记录、缓存与私有令牌会保留。未知文件不会删除；临时卸载工具由系统临时文件清理。", true) { return Ok(()); }
        if install::desktop_running() {
            return Err("install-close-preview-first");
        }
        let _lock = install::lock(&root)?;
        fuze_worker_host::app_registration::preflight(&root)?;
        fuze_worker_host::install_entries::uninstall(
            &root,
            &fuze_worker_host::install_entries::programs()?,
        )?;
        fuze_worker_host::app_registration::remove(&root)?;
        message("Rust 程序、快捷方式与应用登记已移除。用户数据、令牌、未识别文件和安装锁文件保留。Electron 稳定版未改动。", false);
        return Ok(());
    }
    if action == "--install-local" {
        if !message("将当前 Rust 测试包安装到独立用户目录？无需管理员权限，将登记到 Windows 已安装应用，不修改稳定版，不自动启动。安装后可删除解压目录；设置和个人记录独立保留。", true) { return Ok(()); }
        if install::desktop_running() {
            return Err("install-close-preview-first");
        }
        let _lock = install::lock(&root)?;
        fuze_worker_host::app_registration::preflight(&root)?;
        let state = install::read(&root)?;
        let repairing_pending =
            state.pending && state.current.as_ref() == Some(&install::bundled_version(&bundled)?);
        let changed = if repairing_pending {
            false
        } else {
            fuze_worker_host::local_setup::install_with(&root, &bundled, &install::check_bundle)?
        };
        fuze_worker_host::install_entries::create(
            &root,
            &bundled,
            &fuze_worker_host::install_entries::programs()?,
        )?;
        fuze_worker_host::app_registration::register(&root)?;
        message(
            if changed {
                "安装完成，已登记到 Windows 已安装应用。开始菜单已有 Fuze Rust Preview 启动与卸载快捷方式。首次启动通过健康检查后确认新版；此包仍为未签名开发预览。"
            } else {
                "当前版本已安装，启动入口已检查。没有重复安装或启动程序。"
            },
            false,
        );
        return Ok(());
    }
    if action == "--launch" {
        let result = install::launch(&root, &bundled);
        let _ = fuze_worker_host::app_registration::refresh(&root);
        return result;
    }
    if action == "--rollback" {
        if install::desktop_running() {
            return Err("install-close-preview-first");
        }
        let _lock = install::lock(&root)?;
        install::rollback(&root)?;
        fuze_worker_host::app_registration::refresh(&root)?;
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
    let current = match state.current.as_ref() {
        Some(current) => current.clone(),
        None => install::bundled_version(&bundled)?,
    };
    if current.sequence >= release.manifest.sequence
        || current.revision == release.manifest.revision
        || state.rejected.as_ref() == Some(&release.manifest.revision)
    {
        message(
            "当前已是同版/更新版本，或此版本曾启动失败。不会重复安装。",
            false,
        );
        return Ok(());
    }
    if !message(&fuze_worker_host::update_progress::translated("发现 Rust 版本 {}，完整包约 {} MB。下载并校验？有可用基线时优先差分，失败最多回退一次全量。", &[release.manifest.revision.clone(), (release.manifest.size/1024/1024).to_string()]),true){return Ok(());}
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
        // One budget spans map, delta and fallback, rather than restarting per phase.
        let started = std::time::Instant::now();
        let cancelled = || {
            fuze_worker_host::update_progress::cancelled()
                || started.elapsed() > std::time::Duration::from_secs(600)
        };
        fuze_worker_host::update_progress::reset_transfer();
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
            .bytes(&release.map, 8 * 1024 * 1024, &cancelled)
            .or_else(|e| {
                if matches!(
                    e,
                    "update-cancelled-or-timeout"
                        | "update-access-or-release"
                        | "update-redirect-policy"
                        | "update-redirect-limit"
                        | "private-url"
                        | "update-auth-policy"
                ) {
                    Err(e)
                } else {
                    Ok(Vec::new())
                }
            })?;
        fuze_worker_host::update_progress::STAGE.store(2, std::sync::atomic::Ordering::Release);
        let transfer = update_channel::fetch_observed(
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
                    &mut fuze_worker_host::update_progress::Counted(&mut b),
                    &cancelled,
                )?;
                Ok(b)
            },
            &|out| {
                session.archive_observed(
                    &release.archive,
                    release.manifest.size,
                    out,
                    &cancelled,
                    &fuze_worker_host::update_progress::received,
                )
            },
            &cancelled,
            &fuze_worker_host::update_progress::transfer_event,
        )?;
        fuze_worker_host::update_progress::STAGE.store(3, std::sync::atomic::Ordering::Release);
        let extracted = stage.join("version");
        install::unpack(&destination, &extracted, &release.manifest.revision)?;
        install::check_bundle(&extracted)?;
        if fuze_worker_host::update_progress::cancelled() {
            return Err("update-cancelled");
        }
        if install::desktop_running() {
            return Err("install-close-preview-first");
        }
        let summary = fuze_worker_host::update_progress::transfer_summary(
            transfer.mode,
            fuze_worker_host::update_progress::received_bytes(),
        );
        if !message(&fuze_worker_host::update_progress::translated("下载与离线 OCR 自检通过（{}）。是否安装到独立 Rust 目录？保留上一版，下一次启动前再次自检，失败自动回退。", &[summary]),true){return Ok(());}
        if fuze_worker_host::update_progress::cancelled() {
            return Err("update-cancelled");
        }
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
        fuze_worker_host::app_registration::refresh(&root)?;
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
    let wide = |s: &str| {
        fuze_worker_host::i18n::text(s)
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
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
    if std::env::args().nth(1).as_deref() != Some("--verify-package") {
        fuze_worker_host::update_progress::init_locale();
    }
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
        if !fuze_worker_host::update_progress::cancelled() {
            message(&fuze_worker_host::update_progress::translated("操作未完成：{}\n请关闭 Rust 预览版后重试。未知文件会保留；安装或卸载可能部分完成。Electron 稳定版未改动。", &[error.to_string()]),false);
        }
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    std::process::exit(1);
}
