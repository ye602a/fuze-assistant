//! v0.6.3 screen-first policy: persistent stream, bitmap fallback, 60s recovery cooldown.
//! Lazy start: constructing this object never captures. The host gates it to an active match.
use crate::native::{CaptureFeed, DesktopStream, FrameStatus, Result};
use fuze_contract::titles::Atlas;
use std::time::{Duration, Instant};
#[path = "screen_bitmap.rs"]
mod bitmap;

pub trait ScreenDriver {
    fn start(&mut self) -> Result<()>;
    fn stop(&mut self);
    fn stream(&mut self, timeout: Duration) -> Result<FrameStatus>;
    fn bitmap(&mut self) -> Result<()>;
    fn titles(&self, bitmap: bool) -> Result<Atlas<'_>>;
}

#[derive(Default)]
pub struct NativeScreen {
    stream: Option<DesktopStream>,
    bitmap: bitmap::Reader,
}
impl ScreenDriver for NativeScreen {
    fn start(&mut self) -> Result<()> {
        self.stream = Some(DesktopStream::start_primary_screen()?);
        Ok(())
    }
    fn stop(&mut self) {
        self.stream = None;
    }
    fn stream(&mut self, timeout: Duration) -> Result<FrameStatus> {
        self.stream.as_mut().ok_or("capture-missing")?.next(timeout)
    }
    fn bitmap(&mut self) -> Result<()> {
        self.bitmap.read()
    }
    fn titles(&self, bitmap: bool) -> Result<Atlas<'_>> {
        if bitmap {
            self.bitmap.titles()
        } else {
            self.stream.as_ref().ok_or("capture-missing")?.titles()
        }
    }
}

pub struct ScreenCapture<D: ScreenDriver = NativeScreen> {
    driver: D,
    epoch: Instant,
    retry_at: u64,
    waiting_since: u64,
    running: bool,
    saving: bool,
    bitmap: bool,
    ready: bool,
    seen: bool,
    busy: u8,
    generation: u64,
    sequence: u64,
    reason: Option<&'static str>,
}
impl Default for ScreenCapture {
    fn default() -> Self {
        Self::with_driver(NativeScreen::default())
    }
}
impl<D: ScreenDriver> ScreenCapture<D> {
    /// Also used by the isolated end-to-end fixture driver; production always uses NativeScreen.
    pub fn with_driver(driver: D) -> Self {
        Self {
            driver,
            epoch: Instant::now(),
            retry_at: 0,
            waiting_since: 0,
            running: false,
            saving: false,
            bitmap: false,
            ready: false,
            seen: false,
            busy: 0,
            generation: 1,
            sequence: 0,
            reason: None,
        }
    }
    fn failed(&mut self, now: u64, reason: &'static str) {
        self.driver.stop();
        self.running = false;
        self.bitmap = true;
        self.retry_at = now.saturating_add(60_000);
        self.reason = Some(reason);
        self.busy = 0;
    }
    fn accept(&mut self) -> Result<FrameStatus> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("capture-sequence-overflow")?;
        self.ready = true;
        Ok(FrameStatus::Ready)
    }
    fn next_at(&mut self, now: u64, timeout: Duration) -> Result<FrameStatus> {
        self.ready = false;
        if self.saving {
            self.driver.bitmap().map_err(|_| "capture-bitmap")?;
            self.bitmap = true;
            return self.accept();
        }
        if !self.running && now >= self.retry_at {
            match self.driver.start() {
                Ok(()) => {
                    self.running = true;
                    self.waiting_since = now;
                    self.seen = false;
                    self.busy = 0;
                    self.generation = self
                        .generation
                        .checked_add(1)
                        .ok_or("capture-generation-overflow")?;
                }
                Err(_) => self.failed(now, "screen-start"),
            }
        }
        if self.running {
            match self.driver.stream(timeout) {
                Ok(FrameStatus::Ready) => {
                    let usable = self
                        .driver
                        .titles(false)
                        .map(|a| {
                            a.bytes()
                                .chunks_exact(4)
                                .any(|p| u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2]) > 24)
                        })
                        .unwrap_or(false);
                    if usable {
                        self.bitmap = false;
                        self.reason = None;
                        self.waiting_since = now;
                        self.seen = true;
                        self.busy = 0;
                        return self.accept();
                    }
                    self.failed(now, "screen-black-frame");
                }
                Ok(FrameStatus::Resized) => {
                    self.waiting_since = now;
                    self.seen = false;
                    self.generation = self
                        .generation
                        .checked_add(1)
                        .ok_or("capture-generation-overflow")?;
                    return Ok(FrameStatus::Resized);
                }
                Ok(FrameStatus::Busy) => {
                    self.busy = self.busy.saturating_add(1);
                    if self.busy < 5 {
                        return Ok(FrameStatus::Busy);
                    }
                    self.failed(now, "screen-readback");
                }
                Ok(FrameStatus::Timeout | FrameStatus::Duplicate) => {
                    // A static display can stop delivering dirty frames. Periodically verify
                    // through bitmap too, so a silently dead stream cannot block the whole match.
                    let deadline = if self.seen { 8_000 } else { 2_000 };
                    if now.saturating_sub(self.waiting_since) < deadline {
                        // During recovery keep using the functioning bitmap until the stream yields a frame.
                        if !self.bitmap {
                            return Ok(FrameStatus::Timeout);
                        }
                    } else {
                        self.failed(now, "screen-timeout");
                    }
                }
                Ok(FrameStatus::Suspended) | Err(_) => self.failed(now, "screen-frame"),
            }
        }
        self.driver.bitmap().map_err(|_| "capture-bitmap")?;
        self.bitmap = true;
        self.accept()
    }
}
impl<D: ScreenDriver> CaptureFeed for ScreenCapture<D> {
    fn saving(&mut self, saving: bool) {
        if self.saving == saving {
            return;
        }
        self.saving = saving;
        if saving {
            self.driver.stop();
            self.running = false;
            self.ready = false;
            self.bitmap = true;
        }
        // Resume immediately unless an actual capture failure already imposed a cooldown.
        // Saving itself is not a failure and must not delay the first selection.
    }
    fn next(&mut self, timeout: Duration) -> Result<FrameStatus> {
        // Physical monitor coordinates, including a DPI-unaware embedding caller.
        let _dpi = bitmap::PhysicalPixels::new();
        self.next_at(self.epoch.elapsed().as_millis() as u64, timeout)
    }
    fn titles(&self) -> Result<Atlas<'_>> {
        if !self.ready {
            return Err("no-current-frame".into());
        }
        self.driver.titles(self.bitmap)
    }
    fn key(&self) -> (u64, u64) {
        (self.generation, self.sequence)
    }
    fn source(&self) -> &'static str {
        if self.bitmap {
            "screen-bitmap"
        } else if self.running {
            "screen-stream"
        } else {
            "waiting"
        }
    }
    fn fallback(&self) -> Option<&'static str> {
        self.reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Driver {
        start_fail: bool,
        frame: FrameStatus,
        starts: u32,
        bitmaps: u32,
        black: bool,
    }
    impl ScreenDriver for Driver {
        fn start(&mut self) -> Result<()> {
            self.starts += 1;
            if self.start_fail {
                Err("fixture-start".into())
            } else {
                Ok(())
            }
        }
        fn stop(&mut self) {}
        fn stream(&mut self, _: Duration) -> Result<FrameStatus> {
            Ok(self.frame)
        }
        fn bitmap(&mut self) -> Result<()> {
            self.bitmaps += 1;
            Ok(())
        }
        fn titles(&self, _: bool) -> Result<Atlas<'_>> {
            Ok(Atlas::new(
                if self.black { &[0; 12] } else { &[255; 12] },
                1,
                1,
            )?)
        }
    }
    fn capture() -> ScreenCapture<Driver> {
        ScreenCapture::with_driver(Driver {
            start_fail: false,
            frame: FrameStatus::Ready,
            starts: 0,
            bitmaps: 0,
            black: false,
        })
    }
    #[test]
    fn level_one_saving_releases_stream_and_resume_has_no_artificial_cooldown() {
        let mut c = capture();
        c.saving(true);
        c.next_at(0, Duration::ZERO).unwrap();
        c.next_at(8000, Duration::ZERO).unwrap();
        assert_eq!(c.driver.starts, 0);
        assert_eq!(c.driver.bitmaps, 2);
        c.saving(false);
        c.next_at(8250, Duration::ZERO).unwrap();
        assert_eq!(c.source(), "screen-stream");
        c.saving(true);
        assert!(!c.running);
        c.next_at(9000, Duration::ZERO).unwrap();
        assert_eq!(c.driver.starts, 1);
        c.saving(false);
        c.next_at(9250, Duration::ZERO).unwrap();
        assert_eq!(c.source(), "screen-stream");
        assert_eq!(c.driver.starts, 2);
        assert_eq!(c.fallback(), None);
    }
    #[test]
    fn startup_failure_falls_back_immediately_and_recovers_after_cooldown() {
        let mut c = capture();
        c.driver.start_fail = true;
        assert_eq!(c.next_at(0, Duration::ZERO).unwrap(), FrameStatus::Ready);
        assert_eq!(c.source(), "screen-bitmap");
        c.next_at(59_999, Duration::ZERO).unwrap();
        assert_eq!(c.driver.starts, 1);
        c.driver.start_fail = false;
        c.next_at(60_000, Duration::ZERO).unwrap();
        assert_eq!(c.source(), "screen-stream");
        assert_eq!(c.fallback(), None);
        assert_eq!(c.driver.starts, 2);
        assert_eq!(c.driver.bitmaps, 2);
    }
    #[test]
    fn startup_stall_black_frames_and_stalled_active_stream_do_not_wait_forever() {
        let mut c = capture();
        c.driver.frame = FrameStatus::Timeout;
        assert_eq!(c.next_at(0, Duration::ZERO).unwrap(), FrameStatus::Timeout);
        assert_eq!(
            c.next_at(2_000, Duration::ZERO).unwrap(),
            FrameStatus::Ready
        );
        assert_eq!(c.fallback(), Some("screen-timeout"));
        let mut c = capture();
        c.driver.black = true;
        c.next_at(0, Duration::ZERO).unwrap();
        assert_eq!(c.fallback(), Some("screen-black-frame"));
        let mut c = capture();
        c.next_at(0, Duration::ZERO).unwrap();
        c.driver.frame = FrameStatus::Timeout;
        c.next_at(8_000, Duration::ZERO).unwrap();
        assert_eq!(c.source(), "screen-bitmap");
    }
    #[test]
    fn recovering_stream_keeps_bitmap_available_until_first_frame() {
        let mut c = capture();
        c.driver.start_fail = true;
        c.next_at(0, Duration::ZERO).unwrap();
        c.driver.start_fail = false;
        c.driver.frame = FrameStatus::Timeout;
        assert_eq!(
            c.next_at(60_000, Duration::ZERO).unwrap(),
            FrameStatus::Ready
        );
        assert_eq!(c.source(), "screen-bitmap");
        c.driver.frame = FrameStatus::Ready;
        c.next_at(60_500, Duration::ZERO).unwrap();
        assert_eq!(c.source(), "screen-stream");
    }
}
