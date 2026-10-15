//! Explicit maintenance controls. Jobs run only when paused; smoke never touches personal storage.
use super::*;
pub(super) const PREVIEW: usize = 229;
pub(super) const DIAGNOSTICS: usize = 230;
pub(super) const SCAN: usize = 231;
pub(super) const CLEAN: usize = 232;
#[derive(Default)]
pub(super) struct State {
    pub claim: claim_ui::State,
    preview: Option<fuze_native_overlay::native::HiddenOverlay>,
    pub storage: Option<crate::storage::Preview>,
    job: Option<std::sync::mpsc::Receiver<Result<Option<crate::storage::Preview>>>>,
    pub message: Option<&'static str>,
}
impl State {
    pub(super) fn close_preview(&mut self) {
        self.preview = None;
    }
}
pub(super) unsafe fn controls(hwnd: HWND, u: &Ui) {
    claim_ui::controls(hwnd, u);
    for (id, label) in [
        (PREVIEW, "预览推荐浮窗"),
        (DIAGNOSTICS, "保存诊断并打开目录"),
        (SCAN, "扫描存储"),
        (CLEAN, "确认清理预览内容"),
    ] {
        let c = GetDlgItem(hwnd, id as i32);
        ShowWindow(c, if u.settings { SW_SHOWNA } else { SW_HIDE });
        SetWindowTextW(c, wide(label).as_ptr());
        EnableWindow(
            c,
            i32::from(
                !u.smoke
                    && u.desired == PAUSED
                    && u.shown.mode == PAUSED
                    && u.maintenance.job.is_none()
                    && (id != CLEAN
                        || u.maintenance
                            .storage
                            .as_ref()
                            .is_some_and(|p| p.removable > 0)),
            ),
        );
    }
}
pub(super) unsafe fn layout(hwnd: HWND, u: &Ui) {
    claim_ui::layout(hwnd, u);
    if !u.settings {
        return;
    }
    let d = workbench::dpi(hwnd);
    let x = |v| v * d / 96;
    for (id, left, top, w) in [
        (PREVIEW, 48, 1170, 220),
        (DIAGNOSTICS, 284, 1170, 260),
        (SCAN, 48, 1492, 180),
        (CLEAN, 246, 1492, 260),
    ] {
        workbench::place(
            hwnd,
            GetDlgItem(hwnd, id as i32),
            x(left),
            x(top),
            x(w),
            x(34),
            1,
        );
    }
}
pub(super) unsafe fn poll(hwnd: HWND, u: &mut Ui) {
    claim_ui::poll(hwnd, u);
    if u.desired != PAUSED {
        u.maintenance.preview = None;
    }
    if let Some(result) = u
        .maintenance
        .job
        .as_ref()
        .and_then(|rx| match rx.try_recv() {
            Ok(v) => Some(v),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("maintenance-worker")),
            _ => None,
        })
    {
        u.maintenance.job = None;
        match result {
            Ok(Some(p)) => {
                u.maintenance.storage = Some(p);
                u.maintenance.message = Some("存储预览已更新；清理前请确认可回收数量。");
            }
            Ok(None) => {
                u.maintenance.message = Some("已保存脱敏诊断，不包含账号、令牌或原始日志。")
            }
            Err(_) => {
                u.maintenance.storage = None;
                u.maintenance.message = Some("操作未完成；文件可能变化或正被使用，请重新扫描。");
            }
        }
        controls(hwnd, u);
        SendMessageW(hwnd, WM_SIZE, 0, 0);
        InvalidateRect(hwnd, null(), 0);
    }
}
pub(super) unsafe fn command(hwnd: HWND, u: &mut Ui, id: usize) {
    if u.smoke
        || !u.settings
        || u.desired != PAUSED
        || u.shown.mode != PAUSED
        || u.maintenance.job.is_some()
    {
        return;
    }
    if id == PREVIEW {
        if u.maintenance.preview.take().is_some() {
            return;
        }
        let created = (|| {
            let o = fuze_native_overlay::native::HiddenOverlay::with_statistics(
                1,
                "preview".into(),
                Default::default(),
            )?;
            o.locale(crate::i18n::locale())?;
            o.update(fuze_contract::Snapshot {
                schema_version: 1,
                generation: 1,
                sequence: 1,
                dataset_version: "preview".into(),
                selection_confirmed: false,
                actual_stage: fuze_contract::ActualStage::Unknown,
                slots: [
                    fuze_contract::Slot::Known {
                        id: 1,
                        name: crate::i18n::text("测试预览 · 非识别结果").into(),
                        confidence: 1.0,
                    },
                    fuze_contract::Slot::Unreadable,
                    fuze_contract::Slot::Known {
                        id: 3,
                        name: crate::i18n::text("测试预览 · 可关闭窗口").into(),
                        confidence: 1.0,
                    },
                ],
            })?;
            o.visible(true)?;
            Ok::<_, &'static str>(o)
        })();
        u.maintenance.preview = created.ok();
        return;
    }
    let snapshot = serde_json::json!({"schema":1,"mode":u.shown.mode,"connected":matches!(u.shown.connection,Some(Connection::Connected(_))),"capture":u.shown.pipeline.capture,"backend":u.shown.pipeline.backend,"error":u.shown.pipeline.error,"frameAgeSeconds":u.shown.pipeline.frame_age,"actualClaimRound":null,"recommendationScope":"overall","claimProbe":u.maintenance.claim.sanitized()});
    let preview = if id == CLEAN {
        u.maintenance.storage.take()
    } else {
        None
    };
    if id == CLEAN && preview.is_none() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let created = std::thread::Builder::new()
        .name("fuze-maintenance".into())
        .spawn(move || {
            let result = (|| {
                let root = crate::preferences::path()?
                    .parent()
                    .ok_or("maintenance-root")?
                    .to_path_buf();
                match id {
                    SCAN => crate::storage::scan(&root).map(Some),
                    CLEAN => {
                        crate::storage::clean(preview.ok_or("storage-preview-required")?).map(Some)
                    }
                    DIAGNOSTICS => {
                        let dir = root.join("diagnostics");
                        crate::managed_install::safe_dir(&dir)?;
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_err(|_| "diagnostics-clock")?
                            .as_millis();
                        let path = dir.join(format!("status-{now}.json"));
                        let bytes = serde_json::to_vec_pretty(&snapshot)
                            .map_err(|_| "diagnostics-format")?;
                        use std::io::Write;
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(path)
                            .map_err(|_| "diagnostics-write")?
                            .write_all(&bytes)
                            .map_err(|_| "diagnostics-write")?;
                        // Explicit player action; never called by smoke, startup or background collection.
                        let r = unsafe {
                            ShellExecuteW(
                                null_mut(),
                                wide("open").as_ptr(),
                                wide(&dir.to_string_lossy()).as_ptr(),
                                null(),
                                null(),
                                SW_SHOWNORMAL,
                            )
                        };
                        if r as isize <= 32 {
                            return Err("diagnostics-open");
                        }
                        Ok(None)
                    }
                    _ => Err("maintenance-command"),
                }
            })();
            let _ = tx.send(result);
        });
    if created.is_ok() {
        u.maintenance.job = Some(rx);
        u.maintenance.message = Some("正在处理，请稍候…");
    }
    controls(hwnd, u);
    InvalidateRect(hwnd, null(), 0);
}
