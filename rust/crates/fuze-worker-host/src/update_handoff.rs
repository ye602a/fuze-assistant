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
    pub fn from_environment(directory: &std::path::Path) -> Result<Option<Self>> {
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
        let executable = directory.join("fuze-ui-lab.exe");
        let process = owned_process(pid, &executable)?;
        let supervisor = std::env::var("FUZE_DESKTOP_SUPERVISOR")
            .ok()
            .map(|s| s.parse::<u32>().map_err(|_| "update-parent"))
            .transpose()?
            .map(|pid| owned_process(pid, &executable))
            .transpose()?;
        Ok(Some(Self {
            process,
            ready,
            supervisor,
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
        wait_for_exit(&self.process, 15000)?;
        if let Some(supervisor) = self.supervisor {
            wait_for_exit(&supervisor, 15000)?;
        }
        Ok(())
    }
}

// Only handles verified against this installation may be terminated after the grace period.
fn owned_process(pid: u32, executable: &std::path::Path) -> Result<Signal> {
    if pid == 0 || pid == std::process::id() {
        return Err("update-parent");
    }
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return Err("update-parent");
    }
    let process = Signal(handle);
    let mut buffer = [0u16; 32768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err("update-parent");
    }
    let actual = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
    let expected = std::fs::canonicalize(executable).map_err(|_| "update-parent")?;
    if std::fs::canonicalize(actual).ok().as_ref() != Some(&expected) {
        return Err("update-parent");
    }
    Ok(process)
}
fn wait_for_exit(process: &Signal, grace_ms: u32) -> Result<()> {
    unsafe {
        match WaitForSingleObject(process.0, grace_ms) {
            WAIT_OBJECT_0 => return Ok(()),
            WAIT_TIMEOUT => {}
            _ => return Err("update-parent-timeout"),
        }
        if TerminateProcess(process.0, 0) == 0
            || WaitForSingleObject(process.0, 5000) != WAIT_OBJECT_0
        {
            return Err("update-parent-timeout");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_timeout_reaps_only_the_owned_test_process() {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let mut child = Command::new("cmd.exe")
            .args(["/D", "/C", "set /p fuze_test_input="])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let executable = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/cmd.exe");
        assert!(owned_process(child.id(), &std::env::current_exe().unwrap()).is_err());
        let process = owned_process(child.id(), &executable).unwrap();
        wait_for_exit(&process, 100).unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }
    #[test]
    fn ready_handoff_waits_for_both_owned_processes() {
        let _state = crate::update_progress::TEST_STATE.lock().unwrap();
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
