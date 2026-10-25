//! One update session: prepare while the desktop runs, then wait for that exact process.
use crate::Result;
use std::{
    ptr::null,
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{Foundation::*, System::Threading::*};

pub struct Signal(HANDLE);
impl Drop for Signal {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl Signal {
    pub fn create() -> Result<(Self, String)> {
        let name = format!(
            "Local\\FuzeUpdate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "install-clock")?
                .as_nanos()
        );
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateEventW(null(), 1, 0, wide.as_ptr()) };
        if handle.is_null() {
            return Err("update-signal");
        }
        Ok((Self(handle), name))
    }
    pub fn ready(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}

pub struct Parent {
    process: Signal,
    ready: Signal,
    supervisor: Option<Signal>,
}
impl Parent {
    pub fn from_environment() -> Result<Option<Self>> {
        let Ok(name) = std::env::var("FUZE_UPDATE_READY") else {
            return Ok(None);
        };
        let pid: u32 = std::env::var("FUZE_UPDATE_PARENT")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|p| *p > 0)
            .ok_or("update-parent")?;
        if !name.starts_with(&format!("Local\\FuzeUpdate-{pid}-")) {
            return Err("update-signal");
        }
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, wide.as_ptr()) };
        if event.is_null() {
            return Err("update-signal");
        }
        let ready = Signal(event);
        let process = unsafe { OpenProcess(0x00100000, 0, pid) };
        if process.is_null() {
            return Err("update-parent");
        }
        let supervisor = std::env::var("FUZE_DESKTOP_SUPERVISOR")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .map(|pid| unsafe { OpenProcess(0x00100000, 0, pid) });
        if supervisor.is_some_and(|h| h.is_null()) {
            unsafe {
                CloseHandle(process);
            }
            return Err("update-parent");
        }
        Ok(Some(Self {
            process: Signal(process),
            ready,
            supervisor: supervisor.map(Signal),
        }))
    }
    pub fn release(self) -> Result<()> {
        // After this point cancellation must not race with desktop shutdown.
        crate::update_progress::STAGE.store(5, std::sync::atomic::Ordering::Release);
        if crate::update_progress::cancelled() {
            return Err("update-cancelled");
        }
        if unsafe { SetEvent(self.ready.0) } == 0 {
            return Err("update-signal");
        }
        if unsafe { WaitForSingleObject(self.process.0, 30_000) } != WAIT_OBJECT_0 {
            return Err("update-parent-timeout");
        }
        if let Some(supervisor) = self.supervisor {
            if unsafe { WaitForSingleObject(supervisor.0, 30_000) } != WAIT_OBJECT_0 {
                return Err("update-parent-timeout");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ready_handoff_waits_for_both_owned_processes() {
        use std::{
            io::Write,
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        // Two inert stdin readers stand in for the renderer and supervisor. Never launch FUZE.
        let spawn = || {
            Command::new("cmd.exe")
                .args(["/D", "/C", "set /p fuze_test_input="])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x08000000)
                .spawn()
                .unwrap()
        };
        let mut renderer = spawn();
        let mut supervisor = spawn();
        let handle = |pid| {
            let h = unsafe { OpenProcess(0x00100000, 0, pid) };
            assert!(!h.is_null());
            Signal(h)
        };
        let (ready, _) = Signal::create().unwrap();
        let event = ready.0 as usize;
        let parent = Parent {
            process: handle(renderer.id()),
            supervisor: Some(handle(supervisor.id())),
            ready,
        };
        assert!(!parent.ready.ready());
        let finish = std::thread::spawn(move || {
            assert_eq!(
                unsafe { WaitForSingleObject(event as HANDLE, 5000) },
                WAIT_OBJECT_0
            );
            renderer
                .stdin
                .take()
                .unwrap()
                .write_all(b"done\r\n")
                .unwrap();
            renderer.wait().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(150));
            supervisor
                .stdin
                .take()
                .unwrap()
                .write_all(b"done\r\n")
                .unwrap();
            supervisor.wait().unwrap();
        });
        let started = std::time::Instant::now();
        parent.release().unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_millis(150));
        finish.join().unwrap();
        crate::update_progress::STAGE.store(0, std::sync::atomic::Ordering::Release);
    }
    #[test]
    fn update_signal_starts_unset_and_is_session_specific() {
        let (one, a) = Signal::create().unwrap();
        let (two, b) = Signal::create().unwrap();
        assert_ne!(a, b);
        assert!(!one.ready());
        assert!(!two.ready());
        unsafe {
            SetEvent(one.0);
        }
        assert!(one.ready());
        assert!(!two.ready());
    }
}
