//! Render only the owned hidden settings component. No desktop capture, credentials or network.
use crate::{DiagnosticState, Workbench};
use slint::ComponentHandle;
use std::path::Path;
pub fn run(output: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(output)?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|a| a.with_active(false).with_visible(false))
        .select()?;
    let before = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let ui = Workbench::new()?;
    crate::slint_probe::connect(&ui);
    let _binding = crate::feedback_upload_bridge::attach(&ui, true);
    ui.set_page(2);
    let state = ui.global::<DiagnosticState>();
    for action in ["toggle", "configure", "remove", "retry", "open"] {
        state.invoke_upload_action(action.into());
        if !state.get_upload_notice().contains("隔离验证") {
            return Err("feedback-offline-action".into());
        }
    }
    state.set_upload_ready(true);
    state.set_upload_notice("仅用于布局验证，不会上传。".into());
    for (width, enabled, status, name) in [
        (
            1120.,
            false,
            "已关闭，不收集新的自动报告；待传摘要保留在本机。\n待传 0 份",
            "disabled",
        ),
        (
            920.,
            true,
            "GitHub 暂时不可达，摘要保留，稍后重试。\n待传 2 份 · 最近报告 #42",
            "queued",
        ),
    ] {
        ui.window().set_size(slint::LogicalSize::new(width, 860.));
        ui.set_scroll_y(-1680.);
        state.set_upload_enabled(enabled);
        state.set_upload_has_report(enabled);
        state.set_upload_status(status.into());
        let pixels = ui.window().take_snapshot()?;
        image::save_buffer(
            output.join(format!("feedback-{name}.png")),
            pixels.as_bytes(),
            pixels.width(),
            pixels.height(),
            image::ColorType::Rgba8,
        )?;
    }
    let unchanged =
        before == unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if !unchanged {
        return Err("feedback-review-focus".into());
    }
    let report = serde_json::json!({"settings_rendered":2,"offline_actions":5,"network_requests":0,"credential_access":false,"desktop_capture":false,"foreground_unchanged":unchanged});
    std::fs::write(
        output.join("feedback-review.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
