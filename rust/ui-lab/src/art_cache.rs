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
    retry: HashMap<String, (Instant, u8)>,
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
            retry: HashMap::new(),
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
                self.offline
                    || !download
                    || self
                        .retry
                        .get(&key)
                        .is_some_and(|(at, attempts)| *attempts >= 3 || Instant::now() < *at),
                cancelled,
            )
        };
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.starts_with("update-"))
        {
            self.failed(&key, Instant::now());
        }
        let pic = Arc::new(result.ok()?);
        if self.memory.len() >= 192 {
            self.memory.clear();
        }
        self.memory.insert(key, pic.clone());
        Some(pic)
    }
    pub fn begin_request(&mut self) {
        self.retry.clear();
    }
    fn failed(&mut self, key: &str, now: Instant) {
        let attempt = self.retry.get(key).map_or(1, |(_, n)| n.saturating_add(1));
        self.retry.insert(
            key.into(),
            (
                now + Duration::from_secs(match attempt {
                    1 => 2,
                    2 => 10,
                    _ => 30,
                }),
                attempt,
            ),
        );
    }
    pub fn next_retry(&self, version: &str, urls: &[String]) -> Option<Instant> {
        urls.iter()
            .filter_map(|url| {
                let key = format!("{version}|{url}");
                if self.memory.contains_key(&key) {
                    return None;
                }
                self.retry
                    .get(&key)
                    .filter(|(_, n)| *n < 3)
                    .map(|(at, _)| *at)
            })
            .min()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_failed_image_does_not_block_other_images_and_retries_are_bounded() {
        let mut c = Cache::new(true);
        let now = Instant::now();
        c.failed("v|a", now);
        assert_eq!(
            c.next_retry("v", &["a".into()]),
            Some(now + Duration::from_secs(2))
        );
        assert_eq!(c.next_retry("v", &["b".into()]), None);
        c.failed("v|a", now);
        c.failed("v|a", now);
        assert_eq!(c.next_retry("v", &["a".into()]), None);
        c.begin_request();
        assert!(c.retry.is_empty());
    }
    #[test]
    fn every_bundled_champion_decodes_offline_including_reported_missing_portraits() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../ui/assets/champion-portraits.json")).unwrap();
        let mut c = Cache::new(true);
        let rows = manifest["portraits"].as_array().unwrap();
        assert_eq!(rows.len(), 173);
        for row in rows {
            let url = format!(
                "https://cdn.dtodo.cn/hextech/champion-icons/{}.png",
                row["id"]
            );
            assert!(c.get(&url, "fixture", false, &|| false).is_some(), "{url}");
        }
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
