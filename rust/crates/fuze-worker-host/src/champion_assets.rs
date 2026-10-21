//! Independent latest-request detail and icon workers. No network/file/decode work on the window or OCR threads.
use crate::Result;
use fuze_data::{champion_detail::Detail, Locale, Root, Scope};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
};
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub presentation: Option<Arc<Detail>>,
    pub version: String,
    pub id: u32,
    pub icons: Vec<String>,
    pub download: bool,
}
#[derive(Clone)]
pub struct Picture {
    pub bgra: Vec<u8>,
}
#[derive(Clone, Default)]
pub struct Reply {
    pub key: Option<Request>,
    pub detail: Option<Arc<Detail>>,
    pub error: Option<&'static str>,
    pub icons: HashMap<String, Arc<Picture>>,
    pub serial: u64,
}
#[derive(Default)]
struct State {
    desired: Option<Request>,
    generation: u64,
    stop: bool,
    reply: Reply,
}
pub struct Service {
    state: Arc<(Mutex<State>, Condvar)>,
}
impl Service {
    pub fn new(roots: Vec<Root>, offline: bool) -> Result<Self> {
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let icons = state.clone();
        std::thread::Builder::new()
            .name("fuze-champion-icons".into())
            .spawn(move || run_icons(icons, offline))
            .map_err(|_| "champion-icons-thread")?;
        let shared = state.clone();
        std::thread::Builder::new()
            .name("fuze-champion-assets".into())
            .spawn(move || {
                let (lock, wake) = &*shared;
                let mut details = HashMap::<(String, u32), Arc<Detail>>::new();
                let mut seen = 0;
                loop {
                    let Ok(mut s) = lock.lock() else { return };
                    while !s.stop && s.generation == seen {
                        s = match wake.wait(s) {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                    if s.stop {
                        return;
                    }
                    seen = s.generation;
                    let Some(req) = s.desired.clone() else {
                        continue;
                    };
                    drop(s);
                    let cancelled = || {
                        lock.lock()
                            .map(|s| s.stop || s.generation != seen)
                            .unwrap_or(true)
                    };
                    let key = (req.version.clone(), req.id);
                    let detail = if let Some(d) = &req.presentation {
                        Ok(d.clone())
                    } else {
                        details
                            .get(&key)
                            .cloned()
                            .map(Ok)
                            .unwrap_or_else(|| load(&roots, &req).map(Arc::new))
                    };
                    if cancelled() {
                        continue;
                    }
                    if let Ok(d) = &detail {
                        if req.presentation.is_none() {
                            if details.len() >= 32 {
                                details.clear();
                            }
                            details.insert(key, d.clone());
                        }
                    }
                    let mut reply = Reply {
                        key: Some(req.clone()),
                        detail: detail.clone().ok(),
                        error: detail.err(),
                        ..Default::default()
                    };
                    publish(lock, seen, &mut reply);
                    wake.notify_all();
                }
            })
            .map_err(|_| {
                if let Ok(mut s) = state.0.lock() {
                    s.stop = true;
                    state.1.notify_all();
                }
                "champion-assets-thread"
            })?;
        Ok(Self { state })
    }
    pub fn request(&self, value: Option<Request>) {
        if let Ok(mut s) = self.state.0.lock() {
            if s.desired != value {
                s.desired = value;
                s.generation = s.generation.wrapping_add(1);
                s.reply = Reply::default();
                self.state.1.notify_all();
            }
        }
    }
    pub fn changed(&self, previous: &Reply) -> Option<Reply> {
        let s = self.state.0.lock().ok()?;
        if s.reply.key.is_none() && s.desired.is_some() {
            return None;
        }
        (s.reply.key != previous.key || s.reply.serial != previous.serial).then(|| s.reply.clone())
    }
    /// Only explicit viewport changes reprioritize images. Details stay in the worker cache.
    pub fn prioritize(&self, icons: Vec<String>) {
        if let Ok(mut s) = self.state.0.lock() {
            if let Some(req) = s.desired.as_mut() {
                if req.icons != icons {
                    req.icons = icons;
                    s.generation = s.generation.wrapping_add(1);
                    self.state.1.notify_all();
                }
            }
        }
    }
}

fn run_icons(shared: Arc<(Mutex<State>, Condvar)>, offline: bool) {
    let (lock, wake) = &*shared;
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("FuzeRustPreview/icon-cache"));
    let mut memory = HashMap::<String, Arc<Picture>>::new();
    let mut network_retry = std::time::Instant::now();
    let mut seen = 0;
    loop {
        let Ok(mut s) = lock.lock() else { return };
        while !s.stop && (s.generation == seen || s.desired.is_none() || s.reply.key != s.desired) {
            s = match wake.wait(s) {
                Ok(s) => s,
                Err(_) => return,
            };
        }
        if s.stop {
            return;
        }
        seen = s.generation;
        let req = s.desired.clone().unwrap();
        let mut reply = s.reply.clone();
        drop(s);
        let cancelled = || {
            lock.lock()
                .map(|s| s.stop || s.generation != seen)
                .unwrap_or(true)
        };
        let mut urls = req.icons.clone();
        if let Some(d) = &reply.detail {
            urls.extend(d.augments.iter().take(24).map(|a| a.icon.clone()));
            for l in &d.lines {
                urls.extend(l.icons.iter().cloned());
            }
        }
        let mut unique = std::collections::HashSet::new();
        urls.retain(|u| unique.insert(u.clone()));
        urls.truncate(96);
        for url in urls {
            if cancelled() {
                break;
            }
            let key = format!("{}|{url}", req.version);
            let pic = memory.get(&key).cloned().or_else(|| {
                let result = picture(
                    &url,
                    &key,
                    if offline { None } else { cache.as_ref() },
                    offline || !req.download || std::time::Instant::now() < network_retry,
                    &cancelled,
                );
                if result
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.starts_with("update-"))
                {
                    network_retry = std::time::Instant::now() + std::time::Duration::from_secs(60);
                }
                result.ok().map(Arc::new)
            });
            if let Some(pic) = pic {
                if memory.len() >= 192 {
                    memory.clear();
                }
                memory.insert(key, pic.clone());
                reply.icons.insert(url, pic);
                publish(lock, seen, &mut reply);
            }
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if let Ok(mut s) = self.state.0.lock() {
            s.stop = true;
            self.state.1.notify_all();
        }
    }
}
fn publish(lock: &Mutex<State>, generation: u64, reply: &mut Reply) {
    if let Ok(mut s) = lock.lock() {
        if s.generation == generation && !s.stop {
            reply.serial += 1;
            s.reply = reply.clone();
        }
    }
}
fn load(roots: &[Root], r: &Request) -> Result<Detail> {
    let (locale, _) = r.version.split_once('@').ok_or("detail-version")?;
    let locale = match locale {
        "zh-CN" => Locale::ZhCn,
        "zh-TW" => Locale::ZhTw,
        "en-US" => Locale::EnUs,
        _ => return Err("detail-locale"),
    };
    let data = fuze_data::current_augments::prepare_identity(
        roots,
        locale,
        Scope::Detail(r.id),
        &r.version,
    )?;
    if data.identity() != r.version {
        return Err("detail-version-changed");
    }
    data.champion_detail(r.id)
}
pub(crate) fn icon_path(url: &str) -> Result<&str> {
    if let Some(p) = url.strip_prefix("https://raw.communitydragon.org") {
        if p == "/16.21/plugins/rcp-be-lol-game-data/global/default/assets/maps/particles/kiwi/magicmissile_small.png" {
            return Ok(p);
        }
        let base = "/16.21/plugins/rcp-be-lol-game-data/global/default/assets/ux/";
        let relative = p.strip_prefix(base).ok_or("icon-path")?;
        let file = relative
            .strip_prefix("kiwi/augments/icons/")
            .or_else(|| relative.strip_prefix("cherry/augments/icons/"))
            .ok_or("icon-path")?;
        if file.len() <= 100
            && file.ends_with(".png")
            && !file.contains("..")
            && file
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        {
            return Ok(p);
        }
        return Err("icon-path");
    }
    let p = url
        .strip_prefix("https://cdn.dtodo.cn")
        .ok_or("icon-origin")?;
    let relative = p.strip_prefix("/hextech/").ok_or("icon-path")?;
    let (kind, file) = relative.split_once('/').ok_or("icon-path")?;
    if ![
        "champion-icons",
        "item-icons",
        "augment-icons",
        "summoner-spell-icons",
    ]
    .contains(&kind)
        || !file.ends_with(".png")
        || file.len() > 100
        || file.is_empty()
        || !file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        || file.contains("..")
    {
        return Err("icon-path");
    }
    Ok(p)
}
pub fn decode(bytes: &[u8]) -> Result<Picture> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "icon-decode")?
        .resize_exact(48, 48, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let mut bgra = image.into_raw();
    for p in bgra.chunks_exact_mut(4) {
        let a = p[3] as u32;
        let r = p[0];
        p[0] = ((p[2] as u32 * a + 26 * (255 - a)) / 255) as u8;
        p[1] = ((p[1] as u32 * a + 17 * (255 - a)) / 255) as u8;
        p[2] = ((r as u32 * a + 6 * (255 - a)) / 255) as u8;
        p[3] = 255;
    }
    Ok(Picture { bgra })
}
/// Bounded public image cache access for native frontends. Call only on a worker thread.
pub fn picture(
    url: &str,
    key: &str,
    cache: Option<&PathBuf>,
    offline: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<Picture> {
    let path = icon_path(url)?;
    let file = cache.and_then(|root| {
        crate::managed_install::safe_dir(root).ok()?;
        let hash = crate::update_io::hex(&crate::update_io::digest(key.as_bytes(), false).ok()?);
        Some(root.join(format!("{hash}.png")))
    });
    if let Some(f) = &file {
        use std::os::windows::fs::MetadataExt;
        if std::fs::symlink_metadata(f)
            .is_ok_and(|m| m.file_attributes() & 0x400 == 0 && m.len() <= 512 * 1024)
        {
            let mut bytes = Vec::new();
            if std::fs::File::open(f)
                .and_then(|f| f.take(512 * 1024 + 1).read_to_end(&mut bytes))
                .is_ok()
            {
                if let Ok(p) = decode(&bytes) {
                    return Ok(p);
                }
            }
        }
    }
    if offline {
        return Err("icon-offline");
    }
    let host = if url.starts_with("https://raw.communitydragon.org/") {
        "raw.communitydragon.org"
    } else {
        "cdn.dtodo.cn"
    };
    let bytes = crate::update_io::get(host, path, 512 * 1024, None, None, cancel)?;
    let pic = decode(&bytes)?;
    if !cancel() {
        if let Some(f) = file {
            // A full cache stops growing; no broad cleanup of user directories or per-frame scans.
            if f.parent()
                .and_then(|p| std::fs::read_dir(p).ok())
                .is_some_and(|e| e.take(256).count() < 256)
            {
                if let Ok(mut out) = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(f)
                {
                    let _ = out.write_all(&bytes);
                }
            }
        }
    }
    Ok(pic)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_client_icons_use_only_the_pinned_asset_directories() {
        let data = fuze_data::current_augments::metadata().unwrap();
        for row in data["data"].as_array().unwrap() {
            assert!(
                icon_path(row["iconUrl"].as_str().unwrap()).is_ok(),
                "{}",
                row["id"]
            );
        }
        for suffix in ["../secret.png", "a.png?x=1", "a/b.png"] {
            assert!(icon_path(&format!("https://raw.communitydragon.org/16.21/plugins/rcp-be-lol-game-data/global/default/assets/ux/kiwi/augments/icons/{suffix}")).is_err());
        }
        assert!(icon_path("https://raw.communitydragon.org/16.22/plugins/rcp-be-lol-game-data/global/default/assets/ux/kiwi/augments/icons/a.png").is_err());
    }
    #[test]
    fn cached_icons_work_without_network_and_versions_are_isolated() {
        let root = std::env::temp_dir().join(format!("fuze-icon-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let url = "https://cdn.dtodo.cn/hextech/item-icons/1.png";
        let key = format!("zh-CN@1.0|{url}");
        let hash = crate::update_io::hex(&crate::update_io::digest(key.as_bytes(), false).unwrap());
        let file = root.join(format!("{hash}.png"));
        image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 100, 50, 255]))
            .save(&file)
            .unwrap();
        let p = picture(url, &key, Some(&root), true, &|| false).unwrap();
        assert_eq!(p.bgra.len(), 48 * 48 * 4);
        assert_eq!(&p.bgra[..4], &[50, 100, 200, 255]);
        assert!(picture(url, "zh-CN@2.0", Some(&root), true, &|| false).is_err());
        assert!(decode(b"invalid png").is_err());
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn reject_untrusted_icon_urls() {
        for s in [
            "https://evil.test/a.png",
            "https://cdn.dtodo.cn.evil/hextech/item-icons/1.png",
            "https://cdn.dtodo.cn/hextech/item-icons/../a.png",
            "https://cdn.dtodo.cn/hextech/item-icons/1.png?token=a",
        ] {
            assert!(icon_path(s).is_err());
        }
        assert!(icon_path("https://cdn.dtodo.cn/hextech/item-icons/1.png").is_ok());
        assert!(icon_path("https://cdn.dtodo.cn/hextech/champion-icons/21.png").is_ok());
    }
    #[test]
    fn stale_publication_cannot_replace_new_selection() {
        let lock = Mutex::new(State {
            generation: 2,
            ..Default::default()
        });
        let mut r = Reply::default();
        publish(&lock, 1, &mut r);
        assert_eq!(lock.lock().unwrap().reply.serial, 0);
        publish(&lock, 2, &mut r);
        assert_eq!(lock.lock().unwrap().reply.serial, 1);
    }
}
