//! Explicit opt-in native discovery; never invoked by offline prototypes or tests.
//! Only process metadata and its adjacent lockfile. No command-line dump, memory reads or shell.
use crate::{
    reconnect::{Reconnect, State},
    transport::ReadOnlyClient,
    wipe, Credentials, Error, Result,
};
use std::{
    fs::File,
    io::Read,
    mem::{size_of, zeroed},
    path::PathBuf,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Diagnostics::ToolHelp::*, Threading::*},
};
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Secret(Vec<u8>);
impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}
struct Candidate {
    process: Handle,
    pid: u32,
    credentials: Credentials,
}
impl Candidate {
    fn alive(&self) -> bool {
        // A retained kernel handle binds the original process even if its PID is later reused.
        unsafe { WaitForSingleObject(self.process.0, 0) == WAIT_TIMEOUT }
    }
}
fn discover() -> Result<Option<Candidate>> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(Error::Discovery);
        }
        let snapshot = Handle(snapshot);
        let mut row: PROCESSENTRY32W = zeroed();
        row.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot.0, &mut row) == 0 {
            return Err(Error::Discovery);
        }
        let mut found = None;
        let mut denied = false;
        loop {
            let end = row
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(row.szExeFile.len());
            if String::from_utf16_lossy(&row.szExeFile[..end])
                .eq_ignore_ascii_case("LeagueClient.exe")
            {
                let process = OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    row.th32ProcessID,
                );
                if process.is_null() {
                    denied = true;
                } else {
                    let process = Handle(process);
                    let mut path = vec![0u16; 32768];
                    let mut length = path.len() as u32;
                    if QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut length) == 0
                    {
                        denied = true;
                    } else {
                        let image =
                            PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
                        if image.file_name().is_some_and(|n| {
                            n.to_string_lossy().eq_ignore_ascii_case("LeagueClient.exe")
                        }) {
                            let lock = image.with_file_name("lockfile");
                            let credentials = (|| {
                                // Reserve the complete bounded read up front: reallocating a
                                // partially read secret would leave an unwiped old allocation.
                                let mut secret = Secret(Vec::with_capacity(1025));
                                File::open(lock)
                                    .map_err(|_| Error::Discovery)?
                                    .take(1025)
                                    .read_to_end(&mut secret.0)
                                    .map_err(|_| Error::Discovery)?;
                                let text =
                                    std::str::from_utf8(&secret.0).map_err(|_| Error::AuthInput)?;
                                Credentials::from_lockfile(text, row.th32ProcessID)
                            })();
                            match credentials {
                                Ok(credentials) => {
                                    let candidate = Candidate {
                                        process,
                                        pid: row.th32ProcessID,
                                        credentials,
                                    };
                                    if candidate.alive() {
                                        if found.is_some() {
                                            return Err(Error::AmbiguousClient);
                                        }
                                        found = Some(candidate);
                                    }
                                }
                                Err(_) => denied = true,
                            }
                        }
                    }
                }
            }
            if Process32NextW(snapshot.0, &mut row) == 0 {
                if GetLastError() != ERROR_NO_MORE_FILES {
                    return Err(Error::Discovery);
                }
                break;
            }
        }
        // Do not guess one client when another matching process could not be inspected.
        if denied {
            return Err(Error::Discovery);
        }
        Ok(found)
    }
}
struct Connected {
    process: Handle,
    pid: u32,
    client: ReadOnlyClient,
}
/// Own this on a dedicated control/IO thread. `poll` is synchronous, bounded per WinHTTP
/// operation, not hard-cancellable. Caller schedules `wake_at`; no UI/OCR thread invocation.
pub struct AutoClient {
    connection: Option<Connected>,
    retry: Reconnect,
    epoch: Instant,
}
impl Default for AutoClient {
    fn default() -> Self {
        Self {
            connection: None,
            retry: Reconnect::default(),
            epoch: Instant::now(),
        }
    }
}
impl AutoClient {
    pub fn wake_at(&self) -> Option<u64> {
        self.retry.wake_at()
    }
    pub fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    pub fn pause(&mut self) {
        self.connection = None;
        self.retry.pause();
    }
    pub fn resume(&mut self) {
        self.retry.resume(self.now_ms());
    }
    pub fn poll(&mut self) -> State {
        if !self.retry.due(self.now_ms()) {
            return self.retry.state();
        }
        let result = (|| {
            if let Some(c) = &self.connection {
                if unsafe { WaitForSingleObject(c.process.0, 0) } != WAIT_TIMEOUT {
                    self.connection = None;
                }
            }
            if self.connection.is_none() {
                let Some(c) = discover()? else {
                    return Ok(None);
                };
                self.connection = Some(Connected {
                    process: c.process,
                    pid: c.pid,
                    client: ReadOnlyClient::new(c.credentials)?,
                });
            }
            let c = self.connection.as_ref().ok_or(Error::Discovery)?;
            let phase = c.client.phase(c.pid)?;
            if unsafe { WaitForSingleObject(c.process.0, 0) } != WAIT_TIMEOUT {
                return Err(Error::Discovery);
            }
            Ok(Some(phase))
        })();
        if result.is_err() {
            self.connection = None;
        } // Drops credentials; rediscover on next backoff.
        self.retry.finish(result, self.now_ms());
        self.retry.state()
    }
}
