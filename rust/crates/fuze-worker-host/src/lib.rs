//! One persistent child and one pipe thread. The caller must not run blocking calls on a UI loop.
use fuze_contract::wire::{self, ActiveBackend, Reply, Request};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};
pub type Result<T> = std::result::Result<T, &'static str>;
pub struct Worker {
    child: Child,
    send: Option<SyncSender<Request>>,
    receive: Receiver<Result<Reply>>,
    io: Option<JoinHandle<()>>,
    stopped: bool,
    pub backend: ActiveBackend,
    pub fallback_count: u8,
}
impl Worker {
    pub fn start(executable: &Path, args: &[String], timeout: Duration) -> Result<Self> {
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
        let mut child = command.spawn().map_err(|_| "worker-start-failed")?;
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (send, jobs) = mpsc::sync_channel::<Request>(1);
        let (results, receive) = mpsc::channel();
        let io = thread::spawn(move || {
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
        });
        let mut worker = Self {
            child,
            send: Some(send),
            receive,
            io: Some(io),
            stopped: false,
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
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
