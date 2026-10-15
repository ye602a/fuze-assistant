//! Metadata-only game window discovery. No memory reads, foreground changes or input.
use std::{mem::zeroed, path::Path};
use windows_sys::Win32::{Foundation::*, System::Threading::*, UI::WindowsAndMessaging::*};
type Result<T> = std::result::Result<T, &'static str>;
pub struct GameWindow {
    hwnd: isize,
    pid: u32,
    process: HANDLE,
}
impl Drop for GameWindow {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.process);
        }
    }
}
impl GameWindow {
    pub(crate) fn handle(&self) -> isize {
        self.hwnd
    }
    pub fn valid(&self) -> bool {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(self.hwnd as HWND, &mut pid);
            pid == self.pid
                && WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT
                && IsWindowVisible(self.hwnd as HWND) != 0
        }
    }
    pub fn minimized(&self) -> bool {
        unsafe { IsIconic(self.hwnd as HWND) != 0 }
    }
    pub fn foreground(&self) -> bool {
        unsafe { GetForegroundWindow() == self.hwnd as HWND }
    }
}
struct Search<'a> {
    expected: &'a Path,
    found: Option<GameWindow>,
    ambiguous: bool,
}
unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
    let search = &mut *(param as *mut Search<'_>);
    if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
        return 1;
    }
    let mut rect = zeroed();
    if GetClientRect(hwnd, &mut rect) == 0 || rect.right < 640 || rect.bottom < 360 {
        return 1;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 {
        return 1;
    }
    let process = OpenProcess(
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
        0,
        pid,
    );
    if process.is_null() {
        return 1;
    }
    let target = GameWindow {
        hwnd: hwnd as isize,
        pid,
        process,
    };
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    if QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) == 0 {
        return 1;
    }
    let image = std::path::PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
    let matches = image.canonicalize().ok().is_some_and(|p| {
        p.to_string_lossy()
            .eq_ignore_ascii_case(&search.expected.to_string_lossy())
    });
    if matches && target.valid() {
        if search.found.is_some() {
            search.ambiguous = true;
            return 0;
        }
        search.found = Some(target);
    }
    1
}
/// Root comes from the authenticated LeagueClient process, never a window title or remote URL.
pub fn discover(client_root: &Path) -> Result<Option<GameWindow>> {
    let root = client_root
        .canonicalize()
        .map_err(|_| "game-root-unavailable")?;
    let candidate = root.join("Game").join("League of Legends.exe");
    let Ok(expected) = candidate.canonicalize() else {
        return Ok(None);
    };
    if !expected.starts_with(&root) {
        return Err("game-path-escape");
    }
    let mut search = Search {
        expected: &expected,
        found: None,
        ambiguous: false,
    };
    let ok = unsafe { EnumWindows(Some(visit), &mut search as *mut _ as LPARAM) };
    if search.ambiguous {
        return Err("game-window-ambiguous");
    }
    if ok == 0 {
        return Err("game-window-discovery");
    }
    Ok(search.found)
}
