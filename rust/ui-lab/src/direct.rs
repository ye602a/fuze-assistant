//! Isolated Direct2D/DirectWrite probe; never attaches to another application's window.
use std::{path::Path, time::Instant};
use windows::{
    core::{w, Result},
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
            Gdi::*,
            Imaging::*,
        },
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};
const WIDTH: u32 = 760;
const HEIGHT: u32 = 170;
const BG: u32 = 0x07131e;
const PANEL: u32 = 0x101e2a;
const GOLD: u32 = 0xc8aa6e;
const TEXT: u32 = 0xf0e6d2;
const MUTED: u32 = 0xa9bdc6;
fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
fn color(rgb: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 255) as f32 / 255.,
        g: ((rgb >> 8) & 255) as f32 / 255.,
        b: (rgb & 255) as f32 / 255.,
        a: 1.,
    }
}
fn properties(kind: D2D1_RENDER_TARGET_TYPE, dpi: f32) -> D2D1_RENDER_TARGET_PROPERTIES {
    D2D1_RENDER_TARGET_PROPERTIES {
        r#type: kind,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: dpi,
        dpiY: dpi,
        ..Default::default()
    }
}
struct Com;
impl Com {
    fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
struct Scene {
    factory: ID2D1Factory,
    formats: Vec<IDWriteTextFormat>,
    icons: Vec<image::RgbaImage>,
}
impl Scene {
    fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            let factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut formats = Vec::new();
            for size in [10., 14., 20., 24.] {
                let format = dw.CreateTextFormat(
                    w!("Microsoft YaHei UI"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("zh-cn"),
                )?;
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                formats.push(format);
            }
            let icons = [
                include_bytes!("../ui/assets/courageofthecolossus_large.png").as_slice(),
                include_bytes!("../ui/assets/apexinventor_large.png").as_slice(),
                include_bytes!("../ui/assets/backtobasics_large.png").as_slice(),
            ]
            .into_iter()
            .map(|bytes| image::load_from_memory(bytes).map(|i| i.into_rgba8()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(Self {
                factory,
                formats,
                icons,
            })
        }
    }
}
// Formats and decoded image are cached; only target-dependent resources are recreated.
struct Surface {
    brush: ID2D1SolidColorBrush,
    icons: Vec<ID2D1Bitmap>,
}
impl Surface {
    unsafe fn new(scene: &Scene, target: &ID2D1RenderTarget) -> Result<Self> {
        let brush = target.CreateSolidColorBrush(&color(TEXT), None)?;
        let mut icons = Vec::new();
        for icon in &scene.icons {
            let mut bgra = icon.as_raw().clone();
            for p in bgra.chunks_exact_mut(4) {
                p.swap(0, 2);
                for c in 0..3 {
                    p[c] = ((p[c] as u16 * p[3] as u16 + 127) / 255) as u8;
                }
            }
            let logo = target.CreateBitmap(
                D2D_SIZE_U {
                    width: icon.width(),
                    height: icon.height(),
                },
                Some(bgra.as_ptr().cast()),
                icon.width() * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                },
            )?;
            icons.push(logo);
        }
        Ok(Self { brush, icons })
    }
    unsafe fn fill(&self, target: &ID2D1RenderTarget, area: D2D_RECT_F, rgb: u32) {
        self.brush.SetColor(&color(rgb));
        target.FillRectangle(&area, &self.brush);
    }
    unsafe fn text(
        &self,
        scene: &Scene,
        target: &ID2D1RenderTarget,
        text: &str,
        size: usize,
        area: D2D_RECT_F,
        rgb: u32,
    ) {
        self.brush.SetColor(&color(rgb));
        let utf16: Vec<u16> = text.encode_utf16().collect();
        target.DrawText(
            &utf16,
            &scene.formats[size],
            &area,
            &self.brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    unsafe fn draw(&self, scene: &Scene, target: &ID2D1RenderTarget, partial: bool) -> Result<()> {
        target.BeginDraw();
        target.Clear(Some(&color(BG)));
        self.text(
            scene,
            target,
            "FUZE  海克斯推荐",
            0,
            rect(9., 5., 160., 22.),
            GOLD,
        );
        self.text(
            scene,
            target,
            "整体胜率 · 离线预览，非实际领取",
            0,
            rect(169., 5., 490., 22.),
            MUTED,
        );
        // Close affordance is geometry, independent of font coverage.
        self.brush.SetColor(&color(MUTED));
        target.DrawLine(
            windows_numerics::Vector2 { X: 735., Y: 11. },
            windows_numerics::Vector2 { X: 743., Y: 19. },
            &self.brush,
            1.,
            None,
        );
        target.DrawLine(
            windows_numerics::Vector2 { X: 743., Y: 11. },
            windows_numerics::Vector2 { X: 735., Y: 19. },
            &self.brush,
            1.,
            None,
        );
        for (i, title) in ["巨像的勇气", "尖端发明家", "回归基本功"]
            .iter()
            .enumerate()
        {
            let x = 9. + i as f32 * 250.;
            let best = i == 2;
            self.fill(target, rect(x, 34., 242., 127.), PANEL);
            self.brush
                .SetColor(&color(if best { GOLD } else { 0x52616b }));
            target.DrawRectangle(&rect(x, 34., 242., 127.), &self.brush, 1., None);
            if partial && i == 1 {
                self.text(
                    scene,
                    target,
                    "此位置暂不可读",
                    1,
                    rect(x + 10., 50., 222., 24.),
                    GOLD,
                );
                self.text(
                    scene,
                    target,
                    "可能被遮挡或正在刷新。\n保留卡位，等待下一次识别。",
                    0,
                    rect(x + 10., 84., 222., 46.),
                    MUTED,
                );
                continue;
            }
            self.fill(target, rect(x + 10., 49., 64., 64.), 0x07131e);
            target.DrawBitmap(
                &self.icons[i],
                Some(&rect(x + 12., 51., 60., 60.)),
                1.,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
            self.text(scene, target, title, 1, rect(x + 85., 47., 147., 23.), TEXT);
            for (j, label) in ["选取率", "胜率"].iter().enumerate() {
                let sx = x + 85. + j as f32 * 75.;
                self.text(scene, target, label, 0, rect(sx, 74., 72., 17.), MUTED);
                self.text(
                    scene,
                    target,
                    if j == 0 {
                        "12.0%"
                    } else {
                        ["52.0%", "53.0%", "54.0%"][i]
                    },
                    2,
                    rect(sx, 89., 72., 27.),
                    if best { GOLD } else { TEXT },
                );
            }
            self.text(
                scene,
                target,
                "海克斯推荐 · 示例",
                0,
                rect(x + 10., 119., 134., 18.),
                MUTED,
            );
            self.text(
                scene,
                target,
                ["必选 · 62", "必选 · 72", "优先推荐 · 82"][i],
                0,
                rect(x + 145., 119., 90., 18.),
                if best { GOLD } else { TEXT },
            );
            self.fill(target, rect(x + 10., 141., 222., 8.), 0x24343e);
            self.fill(
                target,
                rect(x + 11., 142., 220. * [0.62, 0.72, 0.82][i], 6.),
                if best { GOLD } else { 0x42c6cc },
            );
        }
        target.EndDraw(None, None)
    }
}
struct Native {
    scene: Scene,
    target: Option<ID2D1HwndRenderTarget>,
    surface: Option<Surface>,
    backend: &'static str,
    paints: u32,
    error: bool,
}
impl Native {
    fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            scene: Scene::new()?,
            target: None,
            surface: None,
            backend: "uninitialized",
            paints: 0,
            error: false,
        })
    }
    unsafe fn draw(&mut self, hwnd: HWND) -> Result<()> {
        match self.draw_once(hwnd) {
            Err(error) if error.code() == D2DERR_RECREATE_TARGET => self.draw_once(hwnd),
            result => result,
        }
    }
    unsafe fn draw_once(&mut self, hwnd: HWND) -> Result<()> {
        if self.target.is_none() {
            let mut bounds = RECT::default();
            GetClientRect(hwnd, &mut bounds)?;
            let dpi = GetDpiForWindow(hwnd) as f32;
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U {
                    width: (bounds.right - bounds.left).max(1) as u32,
                    height: (bounds.bottom - bounds.top).max(1) as u32,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
            };
            let (target, backend) = match self
                .scene
                .factory
                .CreateHwndRenderTarget(&properties(D2D1_RENDER_TARGET_TYPE_HARDWARE, dpi), &hp)
            {
                Ok(target) => (target, "hardware_requested_and_created"),
                Err(_) => (
                    self.scene.factory.CreateHwndRenderTarget(
                        &properties(D2D1_RENDER_TARGET_TYPE_SOFTWARE, dpi),
                        &hp,
                    )?,
                    "software_fallback",
                ),
            };
            self.surface = Some(Surface::new(&self.scene, &target)?);
            self.target = Some(target);
            self.backend = backend;
        }
        let result =
            self.surface
                .as_ref()
                .unwrap()
                .draw(&self.scene, self.target.as_ref().unwrap(), false);
        if let Err(ref error) = result {
            if error.code() == D2DERR_RECREATE_TARGET {
                self.surface = None;
                self.target = None;
            }
        }
        result
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lp.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Native;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, message, wp, lp);
    }
    let state = &mut *pointer;
    match message {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut paint);
            state.paints += 1;
            if state.draw(hwnd).is_err() {
                state.error = true;
            }
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let area = &*(lp.0 as *const RECT);
            state.surface = None;
            state.target = None;
            let _ = SetWindowPos(
                hwnd,
                None,
                area.left,
                area.top,
                area.right - area.left,
                area.bottom - area.top,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(target) = &state.target {
                if target
                    .Resize(&D2D_SIZE_U {
                        width: (lp.0 as u32 & 0xffff).max(1),
                        height: ((lp.0 as u32 >> 16) & 0xffff).max(1),
                    })
                    .is_err()
                {
                    state.surface = None;
                    state.target = None;
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let scale = GetDpiForWindow(hwnd) as f32 / 96.;
            let x = (lp.0 as u32 & 0xffff) as f32 / scale;
            let y = ((lp.0 as u32 >> 16) & 0xffff) as f32 / scale;
            if x >= 720. && x <= WIDTH as f32 && y < 30. {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, message, wp, lp)
        }
        _ => DefWindowProcW(hwnd, message, wp, lp),
    }
}
struct OwnWindow {
    hwnd: HWND,
    state: Box<Native>,
    _dpi: ThreadDpi,
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
impl OwnWindow {
    fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            let mut state = Box::new(Native::new()?);
            let dpi = ThreadDpi(SetThreadDpiAwarenessContext(
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            ));
            let instance = GetModuleHandleW(None)?;
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: w!("FuzeIsolatedD2DProbe"),
                ..Default::default()
            });
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("FuzeIsolatedD2DProbe"),
                w!("符择 · 离线三卡原型"),
                WS_POPUP,
                40,
                40,
                WIDTH as i32,
                HEIGHT as i32,
                None,
                None,
                Some(instance.into()),
                Some((&mut *state as *mut Native).cast()),
            )?;
            let window = Self {
                hwnd,
                state,
                _dpi: dpi,
            };
            let scale = GetDpiForWindow(hwnd) as f32 / 96.;
            SetWindowPos(
                hwnd,
                None,
                40,
                40,
                (WIDTH as f32 * scale) as i32,
                (HEIGHT as f32 * scale) as i32,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )?;
            Ok(window)
        }
    }
}
impl Drop for OwnWindow {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}
pub fn review(output: &Path) -> std::result::Result<serde_json::Value, Box<dyn std::error::Error>> {
    let _com = Com::new()?;
    let scene = Scene::new()?;
    let mut cases = Vec::new();
    unsafe {
        let imaging: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        for dpi in [96u32, 120, 144, 168, 192] {
            let width = WIDTH * dpi / 96;
            let height = HEIGHT * dpi / 96;
            let bitmap = imaging.CreateBitmap(
                width,
                height,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )?;
            let mut props = properties(D2D1_RENDER_TARGET_TYPE_SOFTWARE, dpi as f32);
            props.pixelFormat.alphaMode = D2D1_ALPHA_MODE_PREMULTIPLIED;
            let target = scene.factory.CreateWicBitmapRenderTarget(&bitmap, &props)?;
            let surface = Surface::new(&scene, &target)?;
            surface.draw(&scene, &target, false)?;
            let mut timings = Vec::new();
            for _ in 0..24 {
                let at = Instant::now();
                surface.draw(&scene, &target, false)?;
                timings.push(at.elapsed().as_secs_f64() * 1000.);
            }
            for partial in [false, true] {
                surface.draw(&scene, &target, partial)?;
                let mut pixels = vec![0u8; (width * height * 4) as usize];
                bitmap.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
                for p in pixels.chunks_exact_mut(4) {
                    p.swap(0, 2);
                }
                image::save_buffer(
                    output.join(format!(
                        "direct-{dpi}-{}.png",
                        if partial { "partial" } else { "complete" }
                    )),
                    &pixels,
                    width,
                    height,
                    image::ColorType::Rgba8,
                )?;
            }
            cases.push(serde_json::json!({"dpi":dpi,"render":crate::telemetry::samples(&timings)}));
        }
        let before = GetForegroundWindow();
        let mut native = OwnWindow::new()?;
        let mut timings = Vec::new();
        native.state.draw(native.hwnd)?;
        for _ in 0..24 {
            let at = Instant::now();
            native.state.draw(native.hwnd)?;
            timings.push(at.elapsed().as_secs_f64() * 1000.);
        }
        // Explicit cache recreation is not actual driver/device-loss coverage.
        native.state.surface = None;
        native.state.target = None;
        native.state.draw(native.hwnd)?;
        let styles = GetWindowLongPtrW(native.hwnd, GWL_EXSTYLE) as u32;
        let visible = IsWindowVisible(native.hwnd).as_bool();
        let foreground_unchanged = before == GetForegroundWindow();
        let cpu_start = crate::telemetry::cpu_ms();
        let start = Instant::now();
        let paints = native.state.paints;
        while start.elapsed().as_millis() < 800 {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, Some(native.hwnd), 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let cpu_end = crate::telemetry::cpu_ms();
        let cpu_delta = if cpu_start >= 0. && cpu_end >= cpu_start {
            Some(cpu_end - cpu_start)
        } else {
            None
        };
        let required = WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0;
        let flags = styles & required == required;
        if visible || !flags || native.state.error {
            return Err("hidden window invariant failed".into());
        }
        Ok(
            serde_json::json!({"offscreen_renderer":"wic_software_directwrite","cases":cases,"native_hidden":{"backend":native.state.backend,"render":crate::telemetry::samples(&timings),"noactivate_toolwindow_topmost":flags,"visible":visible,"foreground_unchanged_observed":foreground_unchanged,"idle_wall_ms":start.elapsed().as_secs_f64()*1000.,"idle_process_cpu_ms":cpu_delta,"idle_paints":native.state.paints-paints,"resource_recreation":true},"real_device_loss_verified":false,"live_overlay_verified":false}),
        )
    }
}
pub fn interactive() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _com = Com::new()?;
    let window = OwnWindow::new()?;
    unsafe {
        let _ = ShowWindow(window.hwnd, SW_SHOWNOACTIVATE);
        while IsWindow(Some(window.hwnd)).as_bool() {
            let mut message = MSG::default();
            let status = GetMessageW(&mut message, None, 0, 0).0;
            if status == -1 {
                return Err(windows::core::Error::from_win32().into());
            }
            if status == 0 {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}
