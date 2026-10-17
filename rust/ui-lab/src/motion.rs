//! Short, foreground-only transitions. No timer, renderer loop or game access.
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SystemParametersInfoW, SM_REMOTESESSION, SPI_GETCLIENTAREAANIMATION,
};

static GPU_SESSION: AtomicBool = AtomicBool::new(false);

pub fn configure(gpu: bool) {
    GPU_SESSION.store(gpu, Ordering::Relaxed);
}

pub fn refresh(motion: crate::UiMotion<'_>, focused: bool) {
    let mut animation = 0i32;
    // Read only on focus/visibility events, never on each repaint or host update.
    let allowed = focused
        && GPU_SESSION.load(Ordering::Relaxed)
        && unsafe {
            GetSystemMetrics(SM_REMOTESESSION) == 0
                && SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    (&mut animation as *mut i32).cast(),
                    0,
                ) != 0
                && animation != 0
        };
    motion.set_enabled(allowed);
}
