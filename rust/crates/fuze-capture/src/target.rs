//! Metadata-only game window discovery. No memory reads, foreground changes or input.
use std::{
    mem::zeroed,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Diagnostics::ToolHelp::*, Threading::*},
    UI::WindowsAndMessaging::*,
};
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
    /// Diagnostic images may only include the primary-screen ROI inside the foreground game.
    pub fn covers_primary_selection(&self) -> bool {
        use windows_sys::Win32::Graphics::Gdi::*;
        unsafe {
            if !self.valid() || self.minimized() || !self.foreground() {
                return false;
            }
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let mut rect: RECT = zeroed();
            let mut origin = POINT { x: 0, y: 0 };
            if GetMonitorInfoW(monitor, &mut info) == 0
                || GetClientRect(self.hwnd as HWND, &mut rect) == 0
                || ClientToScreen(self.hwnd as HWND, &mut origin) == 0
            {
                return false;
            }
            let screen = info.rcMonitor;
            let (w, h) = (
                (screen.right - screen.left) as u32,
                (screen.bottom - screen.top) as u32,
            );
            let Ok(titles) = fuze_contract::titles::regions(w, h) else {
                return false;
            };
            let Ok(buttons) = fuze_contract::selection_pixels::buttons(w, h) else {
                return false;
            };
            screen.left + titles[0].x as i32 >= origin.x
                && screen.top + titles[0].y as i32 >= origin.y
                && screen.left + (titles[2].x + titles[2].width) as i32 <= origin.x + rect.right
                && screen.top + (buttons[0].y + buttons[0].height) as i32 <= origin.y + rect.bottom
        }
    }
}
struct Search<'a> {
    expected: &'a [PathBuf],
    candidates: &'a [u32],
    found: Option<GameWindow>,
    ambiguous: bool,
    rejected: Rejections,
}

#[derive(Default)]
struct Rejections {
    filtered: bool,
    access: bool,
    path_unavailable: bool,
    mismatch: bool,
}
impl Rejections {
    fn error(&self) -> Option<&'static str> {
        // Fixed categories only: never persist paths, window titles or OS error text.
        if self.access {
            Some("game-window-access")
        } else if self.path_unavailable {
            Some("game-window-image-unavailable")
        } else if self.mismatch {
            Some("game-window-path-mismatch")
        } else if self.filtered {
            Some("game-window-filtered")
        } else {
            None
        }
    }
}

fn expected_images(client_root: &Path) -> Result<Vec<PathBuf>> {
    let root = client_root.canonicalize().map_err(|_| "game-window-root")?;
    let mut installs = vec![root.clone()];
    // LCU discovery returns the executable's parent, including Install/LeagueClient.
    // Support the corresponding sibling Game directory, not arbitrary ancestor scans.
    if root
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("LeagueClient"))
    {
        if let Some(parent) = root.parent() {
            installs.push(parent.to_owned());
        }
    }
    let mut images = Vec::new();
    for install in installs {
        let candidate = install.join("Game").join("League of Legends.exe");
        let Ok(image) = candidate.canonicalize() else {
            continue;
        };
        if !image.starts_with(&install) {
            return Err("game-window-path-escape");
        }
        if image.is_file() && !images.contains(&image) {
            images.push(image);
        }
    }
    if images.is_empty() {
        Err("game-window-layout")
    } else {
        Ok(images)
    }
}

struct ProcessSnapshot(HANDLE);
impl Drop for ProcessSnapshot {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn candidate_pid(row: &PROCESSENTRY32W) -> Option<u32> {
    let end = row
        .szExeFile
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(row.szExeFile.len());
    (row.th32ProcessID != 0
        && String::from_utf16_lossy(&row.szExeFile[..end])
            .eq_ignore_ascii_case("League of Legends.exe"))
    .then_some(row.th32ProcessID)
}

// Shortlist by OS metadata before opening any process handle. This runs only
// when establishing/recovering capture, never while processing frames.
fn candidate_pids() -> Result<Vec<u32>> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err("game-window-discovery");
        }
        let snapshot = ProcessSnapshot(snapshot);
        let mut row: PROCESSENTRY32W = zeroed();
        row.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot.0, &mut row) == 0 {
            return if GetLastError() == ERROR_NO_MORE_FILES {
                Ok(Vec::new())
            } else {
                Err("game-window-discovery")
            };
        }
        let mut pids = Vec::new();
        loop {
            if let Some(pid) = candidate_pid(&row) {
                pids.push(pid);
            }
            if Process32NextW(snapshot.0, &mut row) == 0 {
                return if GetLastError() == ERROR_NO_MORE_FILES {
                    Ok(pids)
                } else {
                    Err("game-window-discovery")
                };
            }
        }
    }
}

unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
    let search = &mut *(param as *mut Search<'_>);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if !search.candidates.contains(&pid) {
        return 1;
    }
    if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
        search.rejected.filtered = true;
        return 1;
    }
    let mut rect = zeroed();
    if GetClientRect(hwnd, &mut rect) == 0 || rect.right < 640 || rect.bottom < 360 {
        search.rejected.filtered = true;
        return 1;
    }
    // Query path and retain process lifetime only. No VM access or escalation.
    let process = OpenProcess(
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
        0,
        pid,
    );
    if process.is_null() {
        search.rejected.access = true;
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
        search.rejected.path_unavailable = true;
        return 1;
    }
    let image = std::path::PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
    if !image.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .eq_ignore_ascii_case("League of Legends.exe")
    }) {
        search.rejected.mismatch = true;
        return 1; // Avoid filesystem resolution for unrelated applications.
    }
    let Ok(image) = image.canonicalize() else {
        search.rejected.path_unavailable = true;
        return 1;
    };
    let matches = search.expected.iter().any(|expected| {
        image
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
    });
    if !matches {
        search.rejected.mismatch = true;
    } else if target.valid() {
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
    let expected = expected_images(client_root)?;
    let candidates = candidate_pids()?;
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut search = Search {
        expected: &expected,
        candidates: &candidates,
        found: None,
        ambiguous: false,
        rejected: Rejections::default(),
    };
    let ok = unsafe { EnumWindows(Some(visit), &mut search as *mut _ as LPARAM) };
    if search.ambiguous {
        return Err("game-window-ambiguous");
    }
    if ok == 0 {
        return Err("game-window-discovery");
    }
    if search.found.is_none() {
        if let Some(error) = search.rejected.error() {
            return Err(error);
        }
    }
    Ok(search.found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Install(PathBuf);
    impl Install {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "fuze-game-layout-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn game(&self, relative: &str) -> PathBuf {
            let dir = self.0.join(relative).join("Game");
            fs::create_dir_all(&dir).unwrap();
            let file = dir.join("League of Legends.exe");
            fs::write(&file, b"fixture, never executed").unwrap();
            file.canonicalize().unwrap()
        }
    }
    impl Drop for Install {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            assert!(self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("fuze-game-layout-"));
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn flat_client_and_split_leagueclient_layouts_resolve_the_same_game() {
        let install = Install::new();
        let game = install.game("");
        let client = install.0.join("LeagueClient");
        fs::create_dir(&client).unwrap();
        assert!(!client.join("Game/League of Legends.exe").exists()); // Previous production assumption.
        assert_eq!(expected_images(&install.0).unwrap(), [game.clone()]);
        assert_eq!(expected_images(&client).unwrap(), [game]);
    }

    #[test]
    fn split_layout_allows_both_known_game_locations_without_picking_arbitrarily() {
        let install = Install::new();
        let sibling = install.game("");
        let nested = install.game("leagueclient");
        assert_eq!(
            expected_images(&install.0.join("leagueclient")).unwrap(),
            [nested, sibling]
        );
    }

    #[test]
    fn layout_search_is_bounded_and_missing_paths_have_fixed_errors() {
        let install = Install::new();
        assert_eq!(expected_images(&install.0), Err("game-window-layout"));
        assert_eq!(
            expected_images(&install.0.join("absent")),
            Err("game-window-root")
        );
        install.game("");
        let unrelated = install.0.join("OtherClient");
        fs::create_dir(&unrelated).unwrap();
        assert_eq!(expected_images(&unrelated), Err("game-window-layout"));
        let nested = install.0.join("Nested/LeagueClient");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(expected_images(&nested), Err("game-window-layout"));
    }

    #[test]
    fn candidate_rejections_are_distinct_from_waiting_for_a_window() {
        assert_eq!(Rejections::default().error(), None);
        for (rejected, expected) in [
            (
                Rejections {
                    filtered: true,
                    ..Default::default()
                },
                "game-window-filtered",
            ),
            (
                Rejections {
                    mismatch: true,
                    ..Default::default()
                },
                "game-window-path-mismatch",
            ),
            (
                Rejections {
                    path_unavailable: true,
                    ..Default::default()
                },
                "game-window-image-unavailable",
            ),
            (
                Rejections {
                    access: true,
                    ..Default::default()
                },
                "game-window-access",
            ),
        ] {
            assert_eq!(rejected.error(), Some(expected));
        }
    }

    #[test]
    fn shortlist_excludes_clients_lookalikes_and_invalid_ids() {
        fn row(pid: u32, name: &str) -> PROCESSENTRY32W {
            let mut row: PROCESSENTRY32W = unsafe { zeroed() };
            row.th32ProcessID = pid;
            for (dest, src) in row.szExeFile.iter_mut().zip(name.encode_utf16()) {
                *dest = src;
            }
            row
        }
        let rows = [
            row(10, "LeagueClient.exe"),
            row(11, "LeagueClientUx.exe"),
            row(12, "League of Legends.exe"),
            row(13, "LEAGUE OF LEGENDS.EXE"),
            row(14, "League of Legends.exe.bak"),
            row(15, "xLeague of Legends.exe"),
            row(0, "League of Legends.exe"),
            row(16, ""),
            row(17, "League of Legends.exe\0unused"),
        ];
        assert_eq!(
            rows.iter().filter_map(candidate_pid).collect::<Vec<_>>(),
            [12, 13, 17]
        );
    }
}
