//! Session-scoped control-thread host shared by offline GPU input and real owned-window WGC.
//! No game discovery/attachment or automatic startup. UI has its own message thread.
use crate::{recovery::Supervisor, Result};
use fuze_capture::native::{FrameStatus, OwnedWindowStream};
use fuze_contract::{
    titles::Atlas,
    wire::{Identity, Reply, Request},
    Snapshot,
};
use fuze_data::Prepared;
use fuze_native_overlay::native::HiddenOverlay;
use fuze_runtime::{
    gate::Selection,
    session::{FrameKey, Phase, Session},
};
use std::time::{Duration, Instant};
pub struct Pipeline {
    session: Session,
    pub worker: Supervisor,
    pub overlay: HiddenOverlay,
    capture: Option<OwnedWindowStream>,
    dataset: String,
    epoch: Instant,
    closed: bool,
    capture_failure: Option<serde_json::Value>,
}
pub struct Delivery {
    pub snapshot: Snapshot,
    pub roi_bytes: usize,
    pub ocr_ms: f64,
    pub total_ms: f64,
}
impl Pipeline {
    pub fn new(worker: Supervisor, data: &Prepared, champion: u32) -> Result<Self> {
        let dataset = data.identity();
        let mut session = Session::new(dataset.clone())?;
        session.update_phase(Phase::Loading)?;
        let overlay = HiddenOverlay::with_statistics(
            session.generation(),
            dataset.clone(),
            data.recommendations(champion)?,
        )?;
        Ok(Self {
            session,
            worker,
            overlay,
            capture: None,
            dataset,
            epoch: Instant::now(),
            closed: false,
            capture_failure: None,
        })
    }
    pub fn cancellation(&self) -> fuze_native_overlay::native::Cancellation {
        self.overlay.cancellation()
    }
    fn cancel_if_requested(&mut self) -> Result<bool> {
        if !self.overlay.cancellation().cancelled() {
            return Ok(false);
        }
        self.capture = None;
        self.worker.stop()?;
        self.closed = true;
        Ok(true)
    }
    pub fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
    pub fn wake_at(&self) -> Option<u64> {
        if self.closed {
            None
        } else {
            self.session.wake_at(self.now())
        }
    }
    pub fn level(&mut self, level: u8) {
        self.session.update_level(level, self.now());
    }
    pub fn phase(&mut self, phase: Phase) -> Result<()> {
        if let Some(cleanup) = self.session.update_phase(phase)? {
            if cleanup.stop_capture {
                self.capture = None;
            }
            if !self
                .overlay
                .reset(cleanup.generation, self.dataset.clone())?
            {
                return Err("overlay-reset");
            }
            if matches!(phase, Phase::Disconnected | Phase::PostGame) {
                self.worker.stop()?;
                self.closed = true;
            }
        }
        Ok(())
    }
    pub fn capture_failure(&self) -> Option<&serde_json::Value> {
        self.capture_failure.as_ref()
    }
    pub fn capture_owned(&mut self, hwnd: isize, visible_ci: bool) -> Result<()> {
        if self.closed || self.wake_at().is_none() {
            return Err("capture-phase");
        }
        self.capture = None;
        let cleanup = self.session.capture_replaced()?;
        if !self
            .overlay
            .reset(cleanup.generation, self.dataset.clone())?
        {
            return Err("overlay-reset");
        }
        self.capture_failure = None;
        match OwnedWindowStream::start(hwnd, visible_ci) {
            Ok(stream) => self.capture = Some(stream),
            Err(error) => {
                self.capture_failure = Some(fuze_capture::native::failure_details(&*error));
                return Err("capture-start");
            }
        }
        Ok(())
    }
    pub fn recognize(&mut self, key: FrameKey, atlas: &Atlas<'_>) -> Result<Option<Delivery>> {
        if self.closed || self.cancel_if_requested()? {
            return Ok(None);
        }
        let Some(ticket) = self.session.begin(key, self.now()) else {
            return Ok(None);
        };
        let started = Instant::now();
        let request = Request::from_atlas(
            Identity {
                generation: ticket.generation,
                sequence: ticket.sequence,
            },
            atlas,
        );
        let roi_bytes = request.pixels.len();
        let reply = match self.worker.run(request, Duration::from_secs(3)) {
            Ok(reply) => reply,
            Err(error) => {
                self.session.fail(ticket, self.now());
                return Err(error);
            }
        };
        if self.cancel_if_requested()? {
            self.session.fail(ticket, self.now());
            return Ok(None);
        }
        let Reply::Result { slots, ocr_ms, .. } = reply else {
            self.session.fail(ticket, self.now());
            return Err("worker-result");
        };
        let Some(snapshot) = self
            .session
            .finish(ticket, slots, Selection::Unknown, self.now())
        else {
            return Ok(None);
        };
        if !self.overlay.update(snapshot.clone())? {
            return Err("overlay-delivery");
        }
        Ok(Some(Delivery {
            snapshot,
            roi_bytes,
            ocr_ms,
            total_ms: started.elapsed().as_secs_f64() * 1000.,
        }))
    }
    /// At most one capture/readback/inference. Caller sleeps until wake_at or control event.
    pub fn tick_wgc(&mut self) -> Result<Option<Delivery>> {
        if self.cancel_if_requested()? {
            return Ok(None);
        }
        if !self.wake_at().is_some_and(|at| self.now() >= at) {
            return Ok(None);
        }
        let Some(mut capture) = self.capture.take() else {
            return Err("capture-missing");
        };
        let result = (|| match capture
            .next(Duration::from_millis(100))
            .map_err(|_| "capture-frame")?
        {
            FrameStatus::Ready => {
                let key = FrameKey {
                    epoch: capture.generation,
                    sequence: capture.sequence,
                };
                let atlas = capture.titles().map_err(|_| "capture-atlas")?;
                self.recognize(key, &atlas)
            }
            FrameStatus::Resized => {
                let cleanup = self.session.capture_replaced()?;
                if !self
                    .overlay
                    .reset(cleanup.generation, self.dataset.clone())?
                {
                    return Err("overlay-reset");
                }
                Ok(None)
            }
            FrameStatus::Timeout | FrameStatus::Duplicate => Ok(None),
        })();
        // Returned inference failures retain the capture stream; a capture failure stops it.
        if self.closed {
            return result;
        }
        if matches!(result, Err("capture-frame" | "capture-atlas")) {
            let cleanup = self.session.capture_replaced()?;
            self.overlay
                .reset(cleanup.generation, self.dataset.clone())?;
        } else {
            self.capture = Some(capture);
        }
        result
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.capture.take();
        let _ = self.worker.stop();
    }
}
