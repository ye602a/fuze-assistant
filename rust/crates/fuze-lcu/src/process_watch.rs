//! Minimal process identity checks; no command lines, memory reads, or elevated rights.
use crate::{
    directory_auth::{Kind, Observation},
    Error, Result,
};
use std::{
    mem::{size_of, zeroed},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Diagnostics::ToolHelp::*, Threading::*},
};

pub(crate) struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub(crate) enum Watch {
    Retained(Handle),
    Snapshot(Observation),
}
impl Watch {
    pub fn from_observation(p: Observation) -> Self {
        // Synchronize alone often remains available when metadata access is restricted.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, p.pid) };
        if handle.is_null() {
            Self::Snapshot(p)
        } else {
            Self::Retained(Handle(handle))
        }
    }
    pub fn refresh_required(&self) -> bool {
        matches!(self, Self::Snapshot(_))
    }
    pub fn alive(&self) -> bool {
        match self {
            Self::Retained(h) => unsafe { WaitForSingleObject(h.0, 0) == WAIT_TIMEOUT },
            Self::Snapshot(p) => observations(false).is_ok_and(|rows| {
                rows.iter()
                    .any(|r| r.pid == p.pid && r.parent == p.parent && r.kind == p.kind)
            }),
        }
    }
}
pub(crate) fn started(handle: HANDLE) -> Option<SystemTime> {
    unsafe {
        let (mut created, mut exited, mut kernel, mut user): (
            FILETIME,
            FILETIME,
            FILETIME,
            FILETIME,
        ) = zeroed();
        if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return None;
        }
        let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        let ticks = ticks.checked_sub(116444736000000000)?;
        UNIX_EPOCH.checked_add(
            Duration::from_secs(ticks / 10_000_000)
                + Duration::from_nanos((ticks % 10_000_000) * 100),
        )
    }
}
pub(crate) fn observations(metadata: bool) -> Result<Vec<Observation>> {
    unsafe {
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return Err(Error::Discovery);
        }
        let h = Handle(h);
        let mut row: PROCESSENTRY32W = zeroed();
        row.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(h.0, &mut row) == 0 {
            return Err(Error::Discovery);
        }
        let mut result = Vec::new();
        loop {
            let end = row
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(row.szExeFile.len());
            let name = String::from_utf16_lossy(&row.szExeFile[..end]);
            let kind = [Kind::Client, Kind::Ux]
                .into_iter()
                .find(|k| name.eq_ignore_ascii_case(k.name()));
            if let Some(kind) = kind {
                let mut p = Observation {
                    pid: row.th32ProcessID,
                    parent: row.th32ParentProcessID,
                    kind,
                    root: None,
                    started: None,
                };
                if metadata {
                    let query = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, p.pid);
                    if !query.is_null() {
                        let query = Handle(query);
                        let mut path = vec![0u16; 32768];
                        let mut length = path.len() as u32;
                        if QueryFullProcessImageNameW(query.0, 0, path.as_mut_ptr(), &mut length)
                            != 0
                        {
                            p.root =
                                PathBuf::from(String::from_utf16_lossy(&path[..length as usize]))
                                    .parent()
                                    .map(|p| p.to_owned());
                        }
                        p.started = started(query.0);
                    }
                }
                result.push(p);
            }
            if Process32NextW(h.0, &mut row) == 0 {
                if GetLastError() != ERROR_NO_MORE_FILES {
                    return Err(Error::Discovery);
                }
                return Ok(result);
            }
        }
    }
}
