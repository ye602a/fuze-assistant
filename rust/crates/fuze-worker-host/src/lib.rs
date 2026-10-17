//! One persistent child and one pipe thread. The caller must not run blocking calls on a UI loop.
#[cfg(windows)]
pub mod app_registration;
#[cfg(windows)]
pub mod install_entries;
#[cfg(windows)]
pub mod local_setup;
#[cfg(windows)]
pub mod stage_reference;
use fuze_contract::wire::{self, ActiveBackend, Reply, Request};
use std::{
    io::Write,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};
pub type Result<T> = std::result::Result<T, &'static str>;
#[cfg(windows)]
pub mod account_history;
#[cfg(windows)]
mod background_data;
#[cfg(windows)]
pub mod cache_maintenance;
#[cfg(windows)]
mod catalog_session;
#[cfg(windows)]
pub mod champ_select;
#[cfg(windows)]
pub mod champion_assets;
#[cfg(windows)]
pub mod controller;
#[cfg(windows)]
pub mod data_update;
#[cfg(windows)]
pub mod desktop;
#[cfg(windows)]
pub mod history_detail;
pub mod i18n;
#[cfg(windows)]
mod job;
#[cfg(windows)]
pub mod observation_names;
#[cfg(windows)]
pub use job::Job as ChildJob;
#[cfg(windows)]
pub mod capture_review;
#[cfg(windows)]
pub mod github_feedback;
#[cfg(windows)]
pub mod legacy_preferences;
#[cfg(windows)]
pub mod lifecycle_review;
#[cfg(windows)]
pub mod live_level;
#[cfg(windows)]
pub mod local_records;
#[cfg(windows)]
pub mod managed_install;
#[cfg(windows)]
mod overlay_art;
#[cfg(windows)]
pub mod overlay_frame;
#[cfg(windows)]
pub mod pipeline;
#[cfg(windows)]
mod post_game_history;
#[cfg(windows)]
pub mod preferences;
#[cfg(windows)]
pub mod private_update;
#[cfg(windows)]
pub mod record_export;
pub mod recovery;
mod retention;
#[cfg(windows)]
pub mod runtime_log;
#[cfg(windows)]
pub mod update_channel;
#[cfg(windows)]
pub mod update_credential;
#[cfg(windows)]
pub mod update_io;
#[cfg(windows)]
pub mod update_progress;
#[cfg(windows)]
pub mod update_resume;
#[cfg(windows)]
pub mod update_validation;
#[cfg(windows)]
pub mod window_memory;
pub struct Worker {
    child: Child,
    send: Option<SyncSender<Request>>,
    receive: Receiver<Result<Reply>>,
    io: Option<JoinHandle<()>>,
    stopped: bool,
    #[cfg(windows)]
    job: Option<job::Job>,
    pub backend: ActiveBackend,
    pub fallback_count: u8,
}
impl Worker {
    #[cfg(windows)]
    pub(crate) fn usage(&self) -> Option<(u32, u64, u64)> {
        use std::os::windows::io::AsRawHandle;
        let (cpu, memory) = crate::resources::process(self.child.as_raw_handle())?;
        Some((self.child.id(), cpu, memory))
    }

    pub fn start(executable: &Path, args: &[String], timeout: Duration) -> Result<Self> {
        Self::start_with_catalog(executable, args, timeout, None)
    }
    pub fn start_with_catalog(
        executable: &Path,
        args: &[String],
        timeout: Duration,
        catalog: Option<std::sync::Arc<Vec<u8>>>,
    ) -> Result<Self> {
        if catalog
            .as_ref()
            .is_some_and(|b| b.is_empty() || b.len() > 2 * 1024 * 1024)
        {
            return Err("worker-catalog-limit");
        }
        let mut command = Command::new(executable);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        #[cfg(windows)]
        let job = job::Job::new()?;
        let mut child = command.spawn().map_err(|_| "worker-start-failed")?;
        #[cfg(windows)]
        if let Err(error) = job.assign(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (send, jobs) = mpsc::sync_channel::<Request>(1);
        let (results, receive) = mpsc::channel();
        let io = thread::Builder::new()
            .name("fuze-ocr-pipe".into())
            .spawn(move || {
                // The worker cannot initialize its model until ownership is established.
                let startup = (|| -> std::io::Result<()> {
                    if let Some(bytes) = catalog {
                        input.write_all(b"FZH2")?;
                        input.write_all(&(bytes.len() as u32).to_le_bytes())?;
                        input.write_all(&bytes)?;
                    } else {
                        input.write_all(b"FZH1")?;
                    }
                    input.flush()
                })();
                if startup.is_err() {
                    let _ = results.send(Err("worker-start-gate"));
                    return;
                }
                let ready = wire::read_reply(&mut output).map_err(|_| "worker-handshake-failed");
                let valid = ready.is_ok();
                if results.send(ready).is_err() || !valid {
                    return;
                }
                while let Ok(job) = jobs.recv() {
                    let reply = job
                        .write(&mut input)
                        .and_then(|_| wire::read_reply(&mut output))
                        .map_err(|_| "worker-pipe-failed");
                    let valid = reply.is_ok();
                    if results.send(reply).is_err() || !valid {
                        break;
                    }
                }
            })
            .map_err(|_| {
                let _ = child.kill();
                let _ = child.wait();
                "worker-thread-start"
            })?;
        let mut worker = Self {
            child,
            send: Some(send),
            receive,
            io: Some(io),
            stopped: false,
            #[cfg(windows)]
            job: Some(job),
            backend: ActiveBackend::Cpu,
            fallback_count: 0,
        };
        match worker.receive.recv_timeout(timeout) {
            Ok(Ok(Reply::Ready {
                schema: 1,
                backend,
                fallback_count,
            })) if fallback_count <= 1 => {
                worker.backend = backend;
                worker.fallback_count = fallback_count;
                Ok(worker)
            }
            _ => {
                worker.stop()?;
                Err("worker-not-ready")
            }
        }
    }
    pub fn run(&mut self, request: Request, timeout: Duration) -> Result<Reply> {
        request.atlas().map_err(|_| "worker-invalid-frame")?;
        if self.stopped {
            return Err("worker-stopped");
        }
        let id = request.id;
        let result = (|| {
            self.send
                .as_ref()
                .ok_or("worker-stopped")?
                .try_send(request)
                .map_err(|_| "worker-busy")?;
            let reply = self
                .receive
                .recv_timeout(timeout)
                .map_err(|_| "worker-timeout")??;
            match &reply {
                Reply::Result {
                    id: received,
                    ocr_ms,
                    backend,
                    fallback_count,
                    ..
                } if *received == id
                    && ocr_ms.is_finite()
                    && *ocr_ms >= 0.
                    && *fallback_count <= 1 =>
                {
                    self.backend = *backend;
                    self.fallback_count = *fallback_count;
                    Ok(reply)
                }
                _ => Err("worker-invalid-result"),
            }
        })();
        if result.is_err() {
            self.stop()?;
        }
        result
    }
    /// Kill only this owned child; wait before reporting stop or allowing a replacement.
    pub fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.send.take();
        #[cfg(windows)]
        self.job.take(); // Also terminates any descendants if the owned worker ever creates them.
        if self
            .child
            .try_wait()
            .map_err(|_| "worker-reap-failed")?
            .is_none()
        {
            // A natural exit may race with kill; confirm with try_wait before declaring failure.
            if self.child.kill().is_err()
                && self
                    .child
                    .try_wait()
                    .map_err(|_| "worker-reap-failed")?
                    .is_none()
            {
                return Err("worker-stop-unconfirmed");
            }
        }
        self.child.wait().map_err(|_| "worker-reap-failed")?;
        if let Some(io) = self.io.take() {
            io.join().map_err(|_| "worker-io-failed")?;
        }
        self.stopped = true;
        Ok(())
    }
    pub fn stopped(&self) -> bool {
        self.stopped
    }
    #[cfg(windows)]
    pub fn verify_job_close(&mut self) -> Result<()> {
        use std::os::windows::io::AsRawHandle;
        self.job.take();
        let result = unsafe {
            windows_sys::Win32::System::Threading::WaitForSingleObject(
                self.child.as_raw_handle(),
                2000,
            )
        };
        self.stop()?;
        if result == 0 {
            Ok(())
        } else {
            Err("worker-job-close-failed")
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(windows)]
pub mod storage;

#[cfg(windows)]
pub mod recognition_sources;

#[cfg(windows)]
pub mod resources;

#[cfg(windows)]
pub mod host_config;
