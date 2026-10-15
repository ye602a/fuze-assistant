//! Host-level fallback owns native crashes as well as returned initialization/inference errors.
use crate::{Result, Worker};
use fuze_contract::wire::{ActiveBackend, Reply, Request};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
pub struct Supervisor {
    worker: Option<Worker>,
    executable: PathBuf,
    args: Vec<String>,
    automatic: bool,
    recovered: bool,
    stopped: bool,
    timeout: Duration,
    launches: u8,
    catalog: Option<std::sync::Arc<Vec<u8>>>,
}
impl Supervisor {
    /// The last argument is the backend. Auto uses explicit DML in the child so only this
    /// supervisor owns fallback; worker and host cannot each consume a separate retry.
    pub fn start(executable: &Path, args: &[String], timeout: Duration) -> Result<Self> {
        Self::start_with_catalog(executable, args, timeout, None)
    }
    pub fn start_with_catalog(
        executable: &Path,
        args: &[String],
        timeout: Duration,
        catalog: Option<std::sync::Arc<Vec<u8>>>,
    ) -> Result<Self> {
        let mode = args.last().ok_or("worker-backend")?;
        if !["auto", "cpu", "dml", "dml-strict"].contains(&mode.as_str()) {
            return Err("worker-backend");
        }
        let automatic = mode == "auto";
        let mut s = Self {
            worker: None,
            executable: executable.into(),
            args: args.to_vec(),
            automatic,
            recovered: false,
            stopped: false,
            timeout,
            launches: 0,
            catalog,
        };
        if automatic {
            *s.args.last_mut().unwrap() = "dml".into();
        }
        match s.launch() {
            Ok(()) => Ok(s),
            Err(e) if automatic && e == "worker-not-ready" => {
                s.recover()?;
                Ok(s)
            }
            Err(e) => Err(e),
        }
    }
    fn launch(&mut self) -> Result<()> {
        self.launches += 1;
        self.worker = Some(Worker::start_with_catalog(
            &self.executable,
            &self.args,
            self.timeout,
            self.catalog.clone(),
        )?);
        Ok(())
    }
    fn recover(&mut self) -> Result<()> {
        if self.recovered || self.stopped || !self.automatic {
            return Err("worker-recovery-unavailable");
        }
        if let Some(worker) = &mut self.worker {
            worker.stop()?; // A failed reap blocks replacement.
        }
        self.worker.take();
        self.recovered = true;
        *self.args.last_mut().unwrap() = "cpu".into();
        match self.launch() {
            Ok(()) => Ok(()),
            Err(e) => {
                self.stopped = true;
                Err(e)
            }
        }
    }
    /// On recovery this returns a fixed error, discarding the old frame. The next fresh frame
    /// uses CPU; no stale screenshot buffer is retained for a replay or copied per frame.
    pub fn run(&mut self, request: Request, timeout: Duration) -> Result<Reply> {
        request.atlas().map_err(|_| "worker-invalid-frame")?;
        if self.stopped {
            return Err("worker-stopped");
        }
        let result = self
            .worker
            .as_mut()
            .ok_or("worker-stopped")?
            .run(request, timeout);
        match result {
            Ok(reply) => Ok(reply),
            Err(e) => {
                if self.automatic
                    && !self.recovered
                    && self.worker.as_ref().is_some_and(Worker::stopped)
                {
                    self.recover()?;
                    Err("worker-recovered-use-next-frame")
                } else {
                    self.stopped = true;
                    Err(e)
                }
            }
        }
    }
    pub fn stop(&mut self) -> Result<()> {
        self.stopped = true;
        if let Some(worker) = &mut self.worker {
            worker.stop()?;
        }
        Ok(())
    }
    pub fn launches(&self) -> u8 {
        self.launches
    }
    pub fn recovered(&self) -> bool {
        self.recovered
    }
    pub fn backend(&self) -> Option<ActiveBackend> {
        self.worker.as_ref().map(|w| w.backend)
    }
    pub fn reaped(&self) -> bool {
        self.worker.as_ref().is_none_or(Worker::stopped)
    }
}
