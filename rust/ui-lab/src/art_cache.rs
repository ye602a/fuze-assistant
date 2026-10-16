//! Bounded worker-owned artwork cache. Embedded v0.6.3 art first, then the existing validated cache.
use fuze_worker_host::champion_assets::{self, Picture};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
pub struct Cache {
    memory: HashMap<String, Arc<Picture>>,
    root: Option<PathBuf>,
    offline: bool,
    retry: Instant,
}
impl Cache {
    pub fn new(offline: bool) -> Self {
        Self {
            memory: HashMap::new(),
            root: if offline {
                None
            } else {
                std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .map(|p| p.join("FuzeRustPreview/icon-cache"))
            },
            offline,
            retry: Instant::now(),
        }
    }
    pub fn get(
        &mut self,
        url: &str,
        version: &str,
        download: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Arc<Picture>> {
        if url.is_empty() || cancelled() {
            return None;
        }
        let key = format!("{version}|{url}");
        if let Some(pic) = self.memory.get(&key) {
            return Some(pic.clone());
        }
        let result = if let Some(bytes) = crate::public_art::bytes(url) {
            champion_assets::decode(bytes)
        } else {
            champion_assets::picture(
                url,
                &key,
                self.root.as_ref(),
                self.offline || !download || Instant::now() < self.retry,
                cancelled,
            )
        };
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.starts_with("update-"))
        {
            self.retry = Instant::now() + Duration::from_secs(60);
        }
        let pic = Arc::new(result.ok()?);
        if self.memory.len() >= 192 {
            self.memory.clear();
        }
        self.memory.insert(key, pic.clone());
        Some(pic)
    }
}
pub fn image(pic: &Picture) -> slint::Image {
    let mut rgba = pic.bgra.clone();
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    slint::Image::from_rgba8(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, 48, 48),
    )
}
