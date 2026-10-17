//! A real Skia surface on an always-hidden owned HWND. No desktop capture or system input.
use crate::Workbench;
use i_slint_renderer_skia::{SkiaRenderer, SkiaSharedContext};
use raw_window_handle::*;
use slint::{
    platform::{Platform, Renderer, WindowAdapter, WindowEvent},
    ComponentHandle, PhysicalSize, Window,
};
use std::{
    cell::{Cell, RefCell},
    ffi::CStr,
    num::NonZeroIsize,
    path::Path,
    rc::{Rc, Weak},
    sync::Arc,
    time::Instant,
};
use windows::{
    core::w,
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

struct Native {
    hwnd: isize,
    instance: isize,
}
unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, message, wp, lp)
}
impl Native {
    fn new() -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                style: CS_OWNDC,
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: w!("FuzeSkiaSurfaceProbe"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("FuzeSkiaSurfaceProbe"),
                w!("FUZE hidden renderer probe"),
                WS_POPUP,
                0,
                0,
                1120,
                860,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            Ok(Arc::new(Self {
                hwnd: hwnd.0 as isize,
                instance: instance.0 as isize,
            }))
        }
    }
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }
    fn check_hidden(&self) -> Result<(), Box<dyn std::error::Error>> {
        if unsafe { IsWindowVisible(self.hwnd()).as_bool() } {
            Err("unexpected visible probe".into())
        } else {
            Ok(())
        }
    }
}
impl HasWindowHandle for Native {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let mut raw =
            Win32WindowHandle::new(NonZeroIsize::new(self.hwnd).ok_or(HandleError::Unavailable)?);
        raw.hinstance = NonZeroIsize::new(self.instance);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
    }
}
impl HasDisplayHandle for Native {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd());
        }
    }
}
struct ThreadDpi(DPI_AWARENESS_CONTEXT);
impl Drop for ThreadDpi {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}

struct Adapter {
    window: Window,
    renderer: SkiaRenderer,
    native: Arc<Native>,
    size: Cell<PhysicalSize>,
    dirty: Cell<bool>,
}
impl WindowAdapter for Adapter {
    fn window(&self) -> &Window {
        &self.window
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn request_redraw(&self) {
        self.dirty.set(true);
    }
    fn set_size(&self, size: slint::WindowSize) {
        let size = size.to_physical(self.window.scale_factor());
        self.size.set(size);
        // Only this hidden HWND. Keep it hidden, non-activating and outside the z-order policy.
        unsafe {
            let _ = SetWindowPos(
                self.native.hwnd(),
                None,
                0,
                0,
                size.width as i32,
                size.height as i32,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
            );
        }
        self.window.dispatch_event(WindowEvent::Resized {
            size: size.to_logical(self.window.scale_factor()),
        });
    }
}
impl Adapter {
    fn draw(&self) -> Result<bool, slint::PlatformError> {
        if !self.dirty.replace(false) {
            return Ok(false);
        }
        self.renderer.render()?;
        Ok(true)
    }
}
struct Backend(Rc<Adapter>);
impl Platform for Backend {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}
type GlFinish = unsafe extern "system" fn();
type GlGetString = unsafe extern "system" fn(u32) -> *const u8;

pub fn review(mode: &str, output: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !matches!(mode, "opengl" | "software") {
        return Err("unknown Skia surface".into());
    }
    std::fs::create_dir_all(output)?;
    let before = unsafe { GetForegroundWindow() };
    let _dpi = ThreadDpi(unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
    });
    let native = Native::new()?;
    let context = SkiaSharedContext::default();
    let renderer = if mode == "opengl" {
        SkiaRenderer::default_opengl(&context)
    } else {
        SkiaRenderer::default_software(&context)
    };
    renderer.set_window_handle(
        native.clone(),
        native.clone(),
        PhysicalSize::new(1120, 860),
        None,
    )?;
    let adapter = Rc::new_cyclic(|weak: &Weak<Adapter>| Adapter {
        window: Window::new(weak.clone()),
        renderer,
        native: native.clone(),
        size: Cell::new(PhysicalSize::new(1120, 860)),
        dirty: Cell::new(true),
    });
    slint::platform::set_platform(Box::new(Backend(adapter.clone())))?;
    let ui = Workbench::new()?;
    crate::slint_probe::connect(&ui);
    let gpu = Rc::new(RefCell::new(serde_json::Value::Null));
    let details = gpu.clone();
    let finish: Rc<Cell<Option<GlFinish>>> = Rc::new(Cell::new(None));
    let completion = finish.clone();
    if mode == "opengl" {
        ui.window().set_rendering_notifier(move |state,api| {
            if matches!(state,slint::RenderingState::BeforeRendering) && details.borrow().is_null(){
                if let slint::GraphicsAPI::NativeOpenGL{get_proc_address}=api {
                    let get=get_proc_address(c"glGetString");let done=get_proc_address(c"glFinish");
                    if get.is_null()||done.is_null(){return;}
                    // Functions belong to our current context; glGetString exposes adapter/driver, never account data.
                    let get:GlGetString=unsafe{std::mem::transmute(get)};
                    let string=|kind|unsafe{let p=get(kind);if p.is_null(){String::new()}else{CStr::from_ptr(p.cast()).to_string_lossy().chars().take(160).collect::<String>()}};
                    *details.borrow_mut()=serde_json::json!({"api":"OpenGL","vendor":string(0x1f00),"renderer":string(0x1f01),"version":string(0x1f02)});
                    completion.set(Some(unsafe{std::mem::transmute::<*const std::ffi::c_void,GlFinish>(done)}));
                }
            }
        })?;
    }
    // Logical show only: our adapter has no native show implementation.
    ui.show()?;
    ui.window().set_size(slint::LogicalSize::new(1120., 860.));
    native.check_hidden()?;
    adapter.draw()?;
    if mode == "opengl" && finish.get().is_none() {
        return Err("OpenGL callback was not observed".into());
    }
    let before_present = Rc::new(Cell::new(None));
    let recorded = before_present.clone();
    adapter
        .renderer
        .set_pre_present_callback(Some(Box::new(move || recorded.set(Some(Instant::now())))));
    let mut cases = Vec::new();
    for (width, height, scale) in [(1120u32, 860u32, 1.0f32), (1120, 860, 1.5), (920, 760, 1.0)] {
        ui.window().dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        ui.window()
            .set_size(slint::LogicalSize::new(width as f32, height as f32));
        adapter.dirty.set(true);
        adapter.draw()?;
        if let Some(done) = finish.get() {
            unsafe {
                done();
            }
        }
        let mut timings = Vec::new();
        let mut before_present_times = Vec::new();
        let mut present_and_finish = Vec::new();
        let cpu = crate::telemetry::cpu_ms();
        for state in 0..12 {
            ui.set_health(state % 4);
            adapter.dirty.set(true);
            before_present.set(None);
            let at = Instant::now();
            adapter.draw()?;
            // Pinned OpenGLSurface keeps this context current through render(). No other GL work occurs here.
            if let Some(done) = finish.get() {
                unsafe {
                    done();
                }
            }
            timings.push(at.elapsed().as_secs_f64() * 1000.);
            if let Some(present) = before_present.get() {
                before_present_times.push(present.duration_since(at).as_secs_f64() * 1000.);
                present_and_finish.push(present.elapsed().as_secs_f64() * 1000.);
            }
        }
        let cpu_end = crate::telemetry::cpu_ms();
        // Drain only redraws requested during layout/rendering, then verify no periodic redraw.
        for _ in 0..3 {
            if !adapter.draw()? {
                break;
            }
        }
        let mut idle_redraws = 0;
        for _ in 0..20 {
            if adapter.draw()? {
                idle_redraws += 1;
            }
        }
        if idle_redraws != 0 {
            return Err("Skia surface continuous redraw".into());
        }
        if before_present_times.len() != 12 {
            return Err("missing pre-present callback".into());
        }
        cases.push(serde_json::json!({"logical_width":width,"scale":scale,"render_and_finish":crate::telemetry::samples(&timings),"before_present":crate::telemetry::samples(&before_present_times),"present_and_finish":crate::telemetry::samples(&present_and_finish),"process_cpu_ms":if cpu>=0. && cpu_end>=cpu{Some(cpu_end-cpu)}else{None},"idle_redraws":idle_redraws}));
    }
    // Real hidden Skia surface, bounded motion samples. Never interpreted as game FPS.
    ui.window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    ui.window().set_size(slint::LogicalSize::new(1120., 860.));
    ui.set_health(0);
    adapter.dirty.set(true);
    adapter.draw()?;
    ui.global::<crate::UiMotion>().set_enabled(true);
    let mut motion_timings = Vec::new();
    for state in [1, 2, 3, 0] {
        ui.set_health(state);
        for _ in 0..22 {
            slint::platform::update_timers_and_animations();
            let at = Instant::now();
            if adapter.draw()? {
                if let Some(done) = finish.get() {
                    unsafe {
                        done();
                    }
                }
                motion_timings.push(at.elapsed().as_secs_f64() * 1000.);
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }
    ui.global::<crate::UiMotion>().set_enabled(false);
    slint::platform::update_timers_and_animations();
    adapter.draw()?;
    let mut motion_idle_redraws = 0;
    for _ in 0..20 {
        slint::platform::update_timers_and_animations();
        if adapter.draw()? {
            motion_idle_redraws += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if motion_idle_redraws != 0 || ui.window().has_active_animations() {
        return Err("Skia motion did not settle".into());
    }
    // Explicit renderer/context suspension and re-attachment, not physical device-loss simulation.
    adapter.renderer.suspend()?;
    adapter
        .renderer
        .set_window_handle(native.clone(), native.clone(), adapter.size.get(), None)?;
    adapter.dirty.set(true);
    adapter.draw()?;
    // The old glFinish pointer isn't reused after context recreation.
    finish.set(None);
    native.check_hidden()?;
    ui.hide()?;
    let report = serde_json::json!({"mode":mode,"graphics":gpu.borrow().clone(),"cases":cases,"hidden":true,"foreground_unchanged_observed":before==unsafe{GetForegroundWindow()},"explicit_context_recreation":true,"real_device_loss":false,"visible_presentation_verified":false,"physical_dpi_verified":false,"same_winit_event_loop":false,"game_access":false,"system_input":false});
    let mut report = report;
    report["motion"] = serde_json::json!({"render_and_finish":crate::telemetry::samples(&motion_timings),"idle_redraws":motion_idle_redraws,"visible_presentation_verified":false});
    std::fs::write(
        output.join(format!("skia-surface-{mode}.json")),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
