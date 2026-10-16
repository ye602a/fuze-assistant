//! Opt-in visible acceptance, launched by the player. Never saves entered text or pixels.
use crate::Workbench;
use slint::{
    winit_030::{
        winit::event::{Ime, WindowEvent},
        EventResult, WinitWindowAccessor,
    },
    ComponentHandle,
};
use std::{cell::RefCell, path::Path, rc::Rc, time::Instant};
#[derive(Default)]
struct Counts {
    frames: u64,
    redraw_events: u64,
    resizes: u64,
    dpi_changes: u64,
    focus_changes: u64,
    ime_preedit: u64,
    ime_commit: u64,
    begin: Option<Instant>,
    durations: Vec<f64>,
    graphics: Option<serde_json::Value>,
}
pub fn run(renderer: &str, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if !matches!(renderer, "software" | "skia" | "skia-software")
        || (renderer != "software" && !cfg!(feature = "gpu"))
    {
        return Err("unsupported renderer".into());
    }
    std::fs::create_dir_all(output)?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(renderer.into())
        .select()?;
    let ui = Workbench::new()?;
    crate::slint_probe::connect(&ui);
    ui.set_backend_note(
        format!("手动验证 · 请求后端 {renderer} · 仅保存耗时和事件计数，不保存输入内容").into(),
    );
    let counts = Rc::new(RefCell::new(Counts::default()));
    let events = counts.clone();
    ui.window().on_winit_window_event(move |_, event| {
        let mut state = events.borrow_mut();
        match event {
            WindowEvent::RedrawRequested => state.redraw_events += 1,
            WindowEvent::Resized(_) => state.resizes += 1,
            WindowEvent::ScaleFactorChanged { .. } => state.dpi_changes += 1,
            WindowEvent::Focused(_) => state.focus_changes += 1,
            WindowEvent::Ime(Ime::Preedit(..)) => state.ime_preedit += 1,
            WindowEvent::Ime(Ime::Commit(_)) => state.ime_commit += 1,
            _ => {}
        }
        EventResult::Propagate
    });
    let drawing = counts.clone();
    let notification_supported=ui.window().set_rendering_notifier(move |phase,api| {
        let mut state=drawing.borrow_mut();
        match phase {
            slint::RenderingState::BeforeRendering=>{
                state.begin=Some(Instant::now());
                if state.graphics.is_none(){
                    if let slint::GraphicsAPI::NativeOpenGL{get_proc_address}=api {
                        let ptr=get_proc_address(c"glGetString");
                        if !ptr.is_null(){
                            let get:unsafe extern "system" fn(u32)->*const u8=unsafe{std::mem::transmute(ptr)};
                            let text=|key|unsafe{let p=get(key);if p.is_null(){String::new()}else{std::ffi::CStr::from_ptr(p.cast()).to_string_lossy().chars().take(160).collect::<String>()}};
                            state.graphics=Some(serde_json::json!({"api":"OpenGL","vendor":text(0x1f00),"renderer":text(0x1f01)}));
                        }
                    }
                }
            }
            slint::RenderingState::AfterRendering=>{
                state.frames+=1;
                if let Some(at)=state.begin.take(){if state.durations.len()<512{state.durations.push(at.elapsed().as_secs_f64()*1000.);}}
            }
            _=>{}
        }
    }).is_ok();
    let at = Instant::now();
    let cpu = crate::telemetry::cpu_ms();
    let outcome = ui.run();
    let cpu_end = crate::telemetry::cpu_ms();
    let state = counts.borrow();
    let report = serde_json::json!({"schema":1,"requested_renderer":renderer,"observed_graphics":state.graphics,"render_notification_supported":notification_supported,"render_callbacks":state.frames,"redraw_events":state.redraw_events,"resize_events":state.resizes,"dpi_events":state.dpi_changes,"focus_events":state.focus_changes,"ime_preedit_events":state.ime_preedit,"ime_commit_events":state.ime_commit,"callback_duration_not_present_or_gpu":if state.durations.is_empty(){serde_json::Value::Null}else{crate::telemetry::samples(&state.durations)},"duration_sample_cap":512,"session_wall_ms":at.elapsed().as_secs_f64()*1000.,"session_cpu_ms":if cpu>=0. && cpu_end>=cpu{Some(cpu_end-cpu)}else{None},"closed_without_event_loop_error":outcome.is_ok(),"ime_correctness":"requires_user_confirmation","entered_text_recorded":false,"game_access":false});
    std::fs::write(
        output.join(format!("manual-{renderer}.json")),
        serde_json::to_vec_pretty(&report)?,
    )?;
    outcome?;
    Ok(())
}
