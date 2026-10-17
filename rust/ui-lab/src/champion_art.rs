//! On-demand public artwork, independent of local text loading and OCR.
use fuze_worker_host::champion_assets::Picture;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
#[derive(Clone, PartialEq)]
struct Request {
    version: String,
    urls: Vec<String>,
    download: bool,
}
#[derive(Default)]
struct State {
    generation: u64,
    next: Option<Request>,
    stop: bool,
    reply: Option<(HashMap<String, Arc<Picture>>, bool)>,
}
pub struct Binding {
    shared: Arc<(Mutex<State>, Condvar)>,
    timer: Rc<slint::Timer>,
    wanted: RefCell<Option<Request>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Binding {
    pub fn new(
        offline: bool,
        mut apply: impl FnMut(HashMap<String, slint::Image>) + 'static,
    ) -> Result<Self, &'static str> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let work = shared.clone();
        let thread = std::thread::Builder::new()
            .name("fuze-slint-champion-art".into())
            .spawn(move || {
                let mut cache = crate::art_cache::Cache::new(offline);
                loop {
                    let mut s = match work.0.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    while !s.stop && s.next.is_none() {
                        s = match work.1.wait(s) {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                    if s.stop {
                        return;
                    }
                    let generation = s.generation;
                    let request = s.next.take().unwrap();
                    drop(s);
                    let cancelled = || {
                        work.0
                            .lock()
                            .map(|s| s.stop || s.generation != generation)
                            .unwrap_or(true)
                    };
                    let mut images = HashMap::new();
                    for (index, url) in request.urls.iter().enumerate() {
                        if cancelled() {
                            break;
                        }
                        if let Some(pic) =
                            cache.get(url, &request.version, request.download, &cancelled)
                        {
                            images.insert(url.clone(), pic);
                        }
                        if (index + 1) % 4 == 0 {
                            if let Ok(mut s) = work.0.lock() {
                                if !s.stop && s.generation == generation {
                                    s.reply = Some((images.clone(), false));
                                }
                            }
                        }
                    }
                    if let Ok(mut s) = work.0.lock() {
                        if !s.stop && s.generation == generation {
                            s.reply = Some((images, true));
                        }
                    }
                }
            })
            .map_err(|_| "champion-art-worker")?;
        let timer = Rc::new(slint::Timer::default());
        let pending = shared.clone();
        let tick = Rc::downgrade(&timer);
        let mut converted = HashMap::<String, (Arc<Picture>, slint::Image)>::new();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let reply = pending
                    .0
                    .lock()
                    .ok()
                    .and_then(|mut s| s.reply.take().map(|r| (s.generation, r)));
                if let Some((generation, (images, complete))) = reply {
                    if converted.len() > 192 {
                        converted.clear();
                    }
                    apply(
                        images
                            .into_iter()
                            .map(|(url, p)| {
                                let image = converted
                                    .get(&url)
                                    .filter(|(old, _)| Arc::ptr_eq(old, &p))
                                    .map(|(_, i)| i.clone())
                                    .unwrap_or_else(|| crate::art_cache::image(&p));
                                converted.insert(url.clone(), (p, image.clone()));
                                (url, image)
                            })
                            .collect(),
                    );
                    if complete && pending.0.lock().is_ok_and(|s| s.generation == generation) {
                        if let Some(t) = tick.upgrade() {
                            t.stop();
                        }
                    }
                }
            },
        );
        timer.stop();
        Ok(Self {
            shared,
            timer,
            wanted: RefCell::new(None),
            thread: Some(thread),
        })
    }
    pub fn request(&self, version: String, mut urls: Vec<String>, download: bool) {
        let mut seen = HashSet::new();
        urls.retain(|u| !u.is_empty() && seen.insert(u.clone()));
        urls.truncate(96);
        let request = Request {
            version,
            urls,
            download,
        };
        if self.wanted.borrow().as_ref() == Some(&request) {
            return;
        }
        *self.wanted.borrow_mut() = Some(request.clone());
        if let Ok(mut s) = self.shared.0.lock() {
            s.generation = s.generation.wrapping_add(1);
            s.reply = None;
            s.next = Some(request);
            self.shared.1.notify_one();
        }
        self.timer.restart();
    }
    pub fn cancel(&self) {
        self.timer.stop();
        *self.wanted.borrow_mut() = None;
        if let Ok(mut s) = self.shared.0.lock() {
            s.generation = s.generation.wrapping_add(1);
            s.next = None;
            s.reply = None;
        }
    }
}
impl Drop for Binding {
    fn drop(&mut self) {
        self.timer.stop();
        if let Ok(mut s) = self.shared.0.lock() {
            s.stop = true;
            self.shared.1.notify_one();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
