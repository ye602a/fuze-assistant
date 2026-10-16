//! Explicit opt-in native discovery; never invoked by offline prototypes or tests.
//! Process metadata, adjacent lockfile, then PID-scoped WMI command arguments. No dumps, memory reads or shell.
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
    root: PathBuf,
}
/// Explicit user-requested history IO only; retains the discovered process handle.
pub struct HistorySession {
    process: Handle,
    pid: u32,
    client: ReadOnlyClient,
}
impl HistorySession {
    pub fn open(manual: Option<&std::path::Path>) -> crate::history::Result<Self> {
        let c = discover(manual)
            .map_err(|_| "history-discovery-failed")?
            .ok_or("history-client-unavailable")?;
        if !c.alive() {
            return Err("history-client-unavailable");
        }
        let client = ReadOnlyClient::new(c.credentials).map_err(|_| "history-connect-failed")?;
        Ok(Self {
            process: c.process,
            pid: c.pid,
            client,
        })
    }
    /// Called only after a player's explicit item-set button, never from the monitor.
    pub fn install_item_sets(
        &self,
        champion: u32,
        sets: &[serde_json::Value],
    ) -> crate::history::Result<()> {
        if unsafe { WaitForSingleObject(self.process.0, 0) } != WAIT_TIMEOUT {
            return Err("item-sets-client-unavailable");
        }
        self.client.install_item_sets(self.pid, champion, sets)
    }
    pub fn owner(&self) -> crate::history::Result<crate::history::Owner> {
        if unsafe { WaitForSingleObject(self.process.0, 0) } != WAIT_TIMEOUT {
            return Err("history-client-unavailable");
        }
        self.client.history_owner(self.pid)
    }
    pub fn rows(
        &self,
        owner: &crate::history::Owner,
    ) -> crate::history::Result<Vec<crate::history::Match>> {
        self.rows_page(owner, 0)
    }
    pub fn sgp_rows(
        &self,
        owner: &crate::history::Owner,
        start: u32,
    ) -> crate::history::Result<Vec<crate::history::Match>> {
        if unsafe { WaitForSingleObject(self.process.0, 0) } != WAIT_TIMEOUT {
            return Err("history-client-unavailable");
        }
        let result = self.client.history_sgp(self.pid, owner, start);
        if unsafe { WaitForSingleObject(self.process.0, 0) } != WAIT_TIMEOUT {
            return Err("history-client-unavailable");
        }
        result
    }
    pub fn rows_page(
        &self,
        owner: &crate::history::Owner,
        start: u32,
    ) -> crate::history::Result<Vec<crate::history::Match>> {
        // The host brackets this request with owner()/phase verification.
        // Parsing independently rejects any participant belonging to a different owner.
        if unsafe { WaitForSingleObject(self.process.0, 0) } != WAIT_TIMEOUT {
            return Err("history-client-unavailable");
        }
        let result = self.client.history_rows(self.pid, owner, start);
        if unsafe { WaitForSingleObject(self.process.0, 0) } != WAIT_TIMEOUT {
            return Err("history-client-unavailable");
        }
        result
    }
}
impl Candidate {
    fn alive(&self) -> bool {
        // A retained kernel handle binds the original process even if its PID is later reused.
        unsafe { WaitForSingleObject(self.process.0, 0) == WAIT_TIMEOUT }
    }
}
fn discover_sources(manual: Option<&std::path::Path>) -> Result<Option<Candidate>> {
    match discover_lockfile(manual) {
        Ok(Some(c)) => Ok(Some(c)),
        Err(Error::AmbiguousClient) => Err(Error::AmbiguousClient),
        previous => match discover_command(manual) {
            Ok(None) => previous,
            other => other,
        },
    }
}
fn discover(manual: Option<&std::path::Path>) -> Result<Option<Candidate>> {
    let automatic = discover_sources(None);
    match automatic {
        Ok(Some(c)) => Ok(Some(c)),
        Err(Error::AmbiguousClient) => Err(Error::AmbiguousClient),
        other => {
            if let Some(root) = manual {
                discover_sources(Some(root))
            } else {
                other
            }
        }
    }
}
fn discover_command(manual: Option<&std::path::Path>) -> Result<Option<Candidate>> {
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
                .eq_ignore_ascii_case("LeagueClientUx.exe")
            {
                let p = OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    row.th32ProcessID,
                );
                if p.is_null() {
                    denied = true;
                } else {
                    let process = Handle(p);
                    let mut path = vec![0u16; 32768];
                    let mut length = path.len() as u32;
                    let readable =
                        QueryFullProcessImageNameW(p, 0, path.as_mut_ptr(), &mut length) != 0;
                    let image = if readable {
                        Some(PathBuf::from(String::from_utf16_lossy(
                            &path[..length as usize],
                        )))
                    } else {
                        manual
                            .filter(|r| r.join("LeagueClientUx.exe").is_file())
                            .map(|r| r.join("LeagueClientUx.exe"))
                    };
                    if image.is_none() {
                        denied = true;
                    }
                    if let Some(image) = image {
                        if let Some(root) = manual {
                            if readable
                                && std::fs::canonicalize(root).ok()
                                    != image.parent().and_then(|p| std::fs::canonicalize(p).ok())
                            {
                                return Err(Error::Discovery);
                            }
                        }
                        if !image.file_name().is_some_and(|s| {
                            s.to_string_lossy()
                                .eq_ignore_ascii_case("LeagueClientUx.exe")
                        }) {
                            return Err(Error::Discovery);
                        }
                        let root = image.parent().ok_or(Error::Discovery)?.to_path_buf();
                        if !root.join("LeagueClient.exe").is_file() {
                            return Err(Error::Discovery);
                        }
                        // Tencent may hide WMI command arguments and omit lockfile.
                        // Read only this live UX process's startup log as the last fallback.
                        let credentials = crate::command_line::credentials(row.th32ProcessID)
                            .or_else(|_| {
                                let mut created: FILETIME = zeroed();
                                let mut exited: FILETIME = zeroed();
                                let mut kernel: FILETIME = zeroed();
                                let mut user: FILETIME = zeroed();
                                if GetProcessTimes(
                                    p,
                                    &mut created,
                                    &mut exited,
                                    &mut kernel,
                                    &mut user,
                                ) == 0
                                {
                                    return Err(Error::Discovery);
                                }
                                let ticks = ((created.dwHighDateTime as u64) << 32)
                                    | created.dwLowDateTime as u64;
                                let since_unix = ticks
                                    .checked_sub(116444736000000000)
                                    .ok_or(Error::Discovery)?;
                                let started = std::time::UNIX_EPOCH
                                    + std::time::Duration::from_nanos(since_unix * 100);
                                crate::client_log::credentials(&root, row.th32ProcessID, started)
                            });
                        match credentials {
                            Ok(credentials) => {
                                let candidate = Candidate {
                                    process,
                                    pid: row.th32ProcessID,
                                    credentials,
                                    root,
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
            if Process32NextW(snapshot.0, &mut row) == 0 {
                if GetLastError() != ERROR_NO_MORE_FILES {
                    return Err(Error::Discovery);
                }
                break;
            }
        }
        if denied {
            return Err(Error::Discovery);
        }
        Ok(found)
    }
}
fn discover_lockfile(manual: Option<&std::path::Path>) -> Result<Option<Candidate>> {
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
                    let readable =
                        QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut length)
                            != 0;
                    let image = if readable {
                        Some(PathBuf::from(String::from_utf16_lossy(
                            &path[..length as usize],
                        )))
                    } else {
                        manual
                            .filter(|r| r.join("LeagueClient.exe").is_file())
                            .map(|r| r.join("LeagueClient.exe"))
                    };
                    if image.is_none() {
                        denied = true;
                    }
                    if let Some(image) = image {
                        if let Some(root) = manual {
                            if readable
                                && std::fs::canonicalize(root).ok()
                                    != image.parent().and_then(|p| std::fs::canonicalize(p).ok())
                            {
                                return Err(Error::Discovery);
                            }
                        }
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
                                        root: image.parent().ok_or(Error::Discovery)?.to_path_buf(),
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
    socket: Option<crate::websocket::Socket>,
    socket_retry: u64,
    process: Handle,
    pid: u32,
    client: ReadOnlyClient,
    root: PathBuf,
}
/// Own this on a dedicated control/IO thread. `poll` is synchronous, bounded per WinHTTP
/// operation, not hard-cancellable. Caller schedules `wake_at`; no UI/OCR thread invocation.
pub struct AutoClient {
    manual: Option<PathBuf>,
    locale: Option<crate::locale::Locale>,
    dirty: bool,
    last_read: u64,
    verified_at: Option<Instant>,
    connection: Option<Connected>,
    retry: Reconnect,
    epoch: Instant,
    context: Option<crate::context::Context>,
}
impl Default for AutoClient {
    fn default() -> Self {
        Self {
            manual: None,
            locale: None,
            dirty: false,
            last_read: 0,
            verified_at: None,
            connection: None,
            retry: Reconnect::default(),
            epoch: Instant::now(),
            context: None,
        }
    }
}
impl AutoClient {
    /// Called at most once per verified game/reconnect by the monitor, outside OCR/UI work.
    pub fn journal_owner(&self) -> crate::history::Result<(crate::history::Owner, u64)> {
        let c = self
            .connection
            .as_ref()
            .ok_or("journal-client-unavailable")?;
        if unsafe { WaitForSingleObject(c.process.0, 0) } != WAIT_TIMEOUT {
            return Err("journal-client-unavailable");
        }
        let result = c.client.journal_owner(c.pid)?;
        if unsafe { WaitForSingleObject(c.process.0, 0) } != WAIT_TIMEOUT
            || self.context.and_then(|c| c.observed_game_id()) != Some(result.1)
        {
            return Err("journal-context-changed");
        }
        Ok(result)
    }
    pub fn with_fallback(manual: Option<PathBuf>) -> Self {
        Self {
            manual,
            ..Default::default()
        }
    }
    pub fn verified_at(&self) -> Option<Instant> {
        self.verified_at
    }
    pub fn locale(&self) -> Option<crate::locale::Locale> {
        self.locale
    }
    pub fn event_observed(&self) -> bool {
        self.connection
            .as_ref()
            .and_then(|c| c.socket.as_ref())
            .is_some_and(|s| s.event_observed)
    }
    pub fn context(&self) -> Option<crate::context::Context> {
        self.context
    }
    pub fn game_root(&self) -> Option<&std::path::Path> {
        self.connection.as_ref().map(|c| c.root.as_path())
    }
    pub fn wake_at(&self) -> Option<u64> {
        self.retry.wake_at()
    }
    pub fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    pub fn pause(&mut self) {
        self.locale = None;
        self.context = None;
        self.verified_at = None;
        self.connection = None;
        self.retry.pause();
    }
    pub fn resume(&mut self) {
        self.retry.resume(self.now_ms());
    }
    pub fn poll(&mut self) -> State {
        let now = self.now_ms();
        if let Some(connection) = &mut self.connection {
            if let Some(socket) = &mut connection.socket {
                match socket.poll() {
                    Ok(dirty) => self.dirty |= dirty,
                    Err(_) => {
                        connection.socket = None;
                        connection.socket_retry = now.saturating_add(30000);
                        self.dirty = true;
                    }
                }
            }
        }
        if self.dirty {
            self.retry
                .refresh_at(now.max(self.last_read.saturating_add(100)));
        }
        if !self.retry.due(self.now_ms()) {
            return self.retry.state();
        }
        self.context = None;
        self.verified_at = None;
        self.locale = None;
        self.dirty = false;
        let result = (|| {
            if let Some(c) = &self.connection {
                if unsafe { WaitForSingleObject(c.process.0, 0) } != WAIT_TIMEOUT {
                    self.connection = None;
                }
            }
            if self.connection.is_none() {
                let Some(c) = discover(self.manual.as_deref())? else {
                    return Ok(None);
                };
                self.connection = Some(Connected {
                    socket: None,
                    socket_retry: 0,
                    process: c.process,
                    pid: c.pid,
                    client: ReadOnlyClient::new(c.credentials)?,
                    root: c.root,
                });
            }
            let now = self.now_ms();
            let c = self.connection.as_mut().ok_or(Error::Discovery)?;
            let context = c.client.context(c.pid)?;
            let locale = c.client.locale(c.pid).ok();
            if unsafe { WaitForSingleObject(c.process.0, 0) } != WAIT_TIMEOUT {
                return Err(Error::Discovery);
            }
            self.context = Some(context);
            self.verified_at = Some(Instant::now());
            self.locale = locale;
            if c.socket.is_none() && now >= c.socket_retry {
                c.socket = c.client.websocket().ok();
                c.socket_retry = now.saturating_add(30000);
            }
            Ok(Some(context.phase))
        })();
        if result.is_err() {
            self.connection = None;
        } // Drops credentials; rediscover on next backoff.
        self.retry.finish(result, self.now_ms());
        self.last_read = self.now_ms();
        if self.event_observed() && matches!(self.retry.state(), State::Connected(_)) {
            self.retry.refresh_at(self.last_read.saturating_add(5000));
        }
        self.retry.state()
    }
}
