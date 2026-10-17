//! Metadata-only game window discovery. No memory reads, foreground changes or input.
use std::{mem::zeroed, path::Path};
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
}
struct Search<'a> {
    expected: &'a Path,
    candidates: &'a [u32],
    found: Option<GameWindow>,
    ambiguous: bool,
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
    if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
        return 1;
    }
    let mut rect = zeroed();
    if GetClientRect(hwnd, &mut rect) == 0 || rect.right < 640 || rect.bottom < 360 {
        return 1;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if !search.candidates.contains(&pid) {
        return 1;
    }
    // Query path and retain process lifetime only. No VM access or escalation.
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
    if !image.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .eq_ignore_ascii_case("League of Legends.exe")
    }) {
        return 1; // Avoid filesystem resolution for unrelated applications.
    }
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
    let candidates = candidate_pids()?;
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut search = Search {
        expected: &expected,
        candidates: &candidates,
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

#[cfg(test)]
mod tests {
    use super::*;

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
