//! Explicit public reference only. Not a source of actual claims or automatic ranking.
use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
const LIMIT: usize = 2 * 1024 * 1024;
const FRESH: u64 = 6 * 3600;
const RETAIN: u64 = 7 * 86400;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: u32,
    pub win: f64,
    pub pick: f64,
    pub samples: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u8,
    pub champion: u32,
    pub patch: String,
    pub date: String,
    pub data_path: String,
    pub fetched: u64,
    pub rows: [Vec<Row>; 5],
}
fn patch(v: &str) -> Option<(u32, u32)> {
    let (a, b) = v.split_once('.')?;
    if a.len() != 2
        || !(1..=2).contains(&b.len())
        || !a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some((a.parse().ok()?, b.parse().ok()?))
}
fn date(v: &str) -> bool {
    let b = v.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || b.iter()
            .enumerate()
            .any(|(i, c)| i != 4 && i != 7 && !c.is_ascii_digit())
    {
        return false;
    }
    let y = v[..4].parse::<u32>().unwrap_or(0);
    let m = v[5..7].parse::<usize>().unwrap_or(0);
    let d = v[8..].parse::<u32>().unwrap_or(0);
    let days = [
        0,
        31,
        if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    y >= 2000 && m > 0 && m <= 12 && d > 0 && d <= days[m]
}
fn version(v: &Value) -> Result<(String, String, String)> {
    let p = v["version"]
        .as_str()
        .filter(|p| patch(p).is_some())
        .ok_or("stage-version")?;
    let d = v["dataDate"]
        .as_str()
        .filter(|d| date(d))
        .ok_or("stage-date")?;
    let path = v["dataPath"].as_str().ok_or("stage-path")?;
    let prefix = format!("data/{p}-");
    let (stamp, hash) = path
        .strip_prefix(&prefix)
        .and_then(|s| s.split_once('-'))
        .ok_or("stage-path")?;
    if stamp.len() != 8
        || !stamp.bytes().all(|c| c.is_ascii_digit())
        || hash.len() != 12
        || !hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("stage-path");
    }
    Ok((p.into(), d.into(), path.into()))
}
fn valid_rows(rows: &[Row]) -> bool {
    let mut ids = HashSet::new();
    rows.len() <= 512
        && rows.iter().all(|r| {
            r.id > 0
                && r.id <= 100000
                && ids.insert(r.id)
                && r.samples <= 9007199254740991
                && r.win.is_finite()
                && (0.0..=1.0).contains(&r.win)
                && r.pick.is_finite()
                && (0.0..=1.0).contains(&r.pick)
        })
}
impl Snapshot {
    fn valid(&self, now: u64) -> bool {
        self.schema==1 && self.champion>0 && self.champion<=10000 && self.fetched<=now && now-self.fetched<=RETAIN && !self.rows[0].is_empty() && self.rows.iter().all(|r|valid_rows(r)) && version(&serde_json::json!({"version":self.patch,"dataDate":self.date,"dataPath":self.data_path})).is_ok()
    }
    fn normalize(&mut self) {
        for rows in &mut self.rows {
            rows.sort_by(|a, b| {
                b.win
                    .total_cmp(&a.win)
                    .then(b.samples.cmp(&a.samples))
                    .then(a.id.cmp(&b.id))
            });
        }
    }
    pub fn stale(&self, now: u64) -> bool {
        now.saturating_sub(self.fetched) >= FRESH
    }
    pub fn visible(&self, stage: usize) -> Vec<&Row> {
        self.rows[stage.min(4)]
            .iter()
            .filter(|r| r.samples >= 200)
            .collect()
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
fn json(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > LIMIT {
        return Err("stage-size");
    }
    serde_json::from_slice(bytes).map_err(|_| "stage-format")
}
fn fetch_snapshot(
    id: u32,
    now: u64,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>>,
    cancel: &dyn Fn() -> bool,
) -> Result<Snapshot> {
    if id == 0 || id > 10000 || cancel() {
        return Err("stage-request");
    }
    let manifest = json(&fetch("/data/versions.json")?)?;
    let versions = manifest["versions"]
        .as_array()
        .filter(|v| v.len() <= 100)
        .ok_or("stage-version")?;
    let latest = manifest["latest"]
        .as_str()
        .filter(|p| patch(p).is_some())
        .ok_or("stage-version")?;
    let entries: Vec<_> = versions.iter().filter(|v| v["version"] == latest).collect();
    if entries.len() != 1 {
        return Err("stage-version");
    }
    let (patch, date, data_path) = version(entries[0])?;
    if cancel() {
        return Err("stage-cancelled");
    }
    let data = json(&fetch(&format!(
        "/{data_path}/stats/all/champion-details/{id}.json"
    ))?)?;
    if data["champion"]["id"].as_u64() != Some(id as u64) {
        return Err("stage-champion");
    }
    let mut rows: [Vec<Row>; 5] = Default::default();
    for (stage, target) in rows.iter_mut().enumerate() {
        let value = if stage == 0 {
            &data["augments"]["all"]
        } else {
            &data["augments"]["stages"][stage.to_string()]
        };
        if stage > 0 && value.is_null() {
            continue;
        }
        let list = value
            .as_array()
            .filter(|a| a.len() <= 512)
            .ok_or("stage-rows")?;
        for r in list {
            target.push(Row {
                id: r["id"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or("stage-id")?,
                win: r["winRate"].as_f64().ok_or("stage-rate")?,
                pick: r["pickRate"].as_f64().ok_or("stage-rate")?,
                samples: r["sampleCount"].as_u64().ok_or("stage-samples")?,
            });
        }
    }
    let mut s = Snapshot {
        schema: 1,
        champion: id,
        patch,
        date,
        data_path,
        fetched: now,
        rows,
    };
    if !s.valid(now) {
        return Err("stage-format");
    }
    s.normalize();
    Ok(s)
}
#[derive(Clone, Default)]
pub struct Reply {
    pub snapshot: Option<Arc<Snapshot>>,
    pub names: Arc<HashMap<u32, String>>,
    pub cached: bool,
    pub issue: Option<&'static str>,
    pub serial: u64,
}
#[derive(Clone)]
pub struct Request {
    pub id: u32,
    pub locale: fuze_data::Locale,
    pub refresh: bool,
}
#[derive(Default)]
struct State {
    desired: Option<Request>,
    generation: u64,
    stop: bool,
    reply: Reply,
}
pub struct Service {
    shared: Arc<(Mutex<State>, Condvar)>,
}
struct Cache {
    entries: VecDeque<Arc<Snapshot>>,
    path: Option<PathBuf>,
    retry: u64,
}
impl Cache {
    fn new(path: Option<PathBuf>) -> Self {
        let mut entries = VecDeque::new();
        if let Some(p) = &path {
            if let Ok(bytes) = crate::update_channel::bounded(p, LIMIT as u64) {
                if let Ok(list) = serde_json::from_slice::<Vec<Snapshot>>(&bytes) {
                    if list.len() <= 8 {
                        let mut ids = HashSet::new();
                        for mut s in list {
                            if s.valid(now()) && ids.insert(s.champion) {
                                s.normalize();
                                entries.push_back(Arc::new(s));
                            }
                        }
                    }
                }
            }
        }
        Self {
            entries,
            path,
            retry: 0,
        }
    }
    fn get(
        &mut self,
        id: u32,
        refresh: bool,
        at: u64,
        fetch: &dyn Fn(&str) -> Result<Vec<u8>>,
        cancel: &dyn Fn() -> bool,
    ) -> Reply {
        let previous = self
            .entries
            .iter()
            .find(|s| s.champion == id && s.valid(at))
            .cloned();
        let mut reply = Reply {
            snapshot: previous.clone(),
            cached: previous.is_some(),
            ..Default::default()
        };
        if cancel() {
            reply.issue = Some("stage-cancelled");
            return reply;
        }
        if previous.is_some() && !refresh {
            return reply;
        }
        if at < self.retry {
            reply.issue = Some("stage-busy");
            return reply;
        }
        self.retry = at.saturating_add(30);
        match fetch_snapshot(id, at, fetch, cancel) {
            Ok(s)
                if !cancel()
                    && previous.as_ref().is_none_or(|old| {
                        patch(&s.patch) > patch(&old.patch)
                            || s.patch == old.patch && s.date >= old.date
                    }) =>
            {
                let s = Arc::new(s);
                self.entries
                    .retain(|old| old.champion != id && old.valid(at));
                self.entries.push_back(s.clone());
                while self.entries.len() > 8 {
                    self.entries.pop_front();
                }
                reply.snapshot = Some(s);
                reply.cached = false;
                if let Some(path) = &self.path {
                    let list: Vec<_> = self.entries.iter().map(|s| s.as_ref()).collect();
                    if !cancel() {
                        if serde_json::to_vec(&list).is_ok_and(|b| b.len() <= LIMIT) {
                            if crate::preferences::save_json(path, &list).is_err() {
                                reply.issue = Some("stage-cache-write")
                            }
                        } else {
                            reply.issue = Some("stage-cache-limit")
                        }
                    }
                }
            }
            _ => {
                self.retry = at.saturating_add(300);
                reply.issue = Some("stage-unavailable");
            }
        }
        reply
    }
}
impl Service {
    pub fn new(roots: Vec<fuze_data::Root>, offline: bool) -> Result<Self> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("fuze-stage-reference".into())
            .spawn(move || {
                let mut cache = None;
                let mut seen = 0;
                loop {
                    let (lock, wake) = &*worker;
                    let Ok(mut s) = lock.lock() else { return };
                    while !s.stop && (s.desired.is_none() || s.generation == seen) {
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
                    let cancel = || {
                        lock.lock()
                            .map(|s| s.stop || s.generation != seen)
                            .unwrap_or(true)
                    };
                    if cancel() {
                        continue;
                    }
                    let mut reply = if offline {
                        Reply {
                            issue: Some("stage-offline"),
                            ..Default::default()
                        }
                    } else {
                        let cache = cache.get_or_insert_with(|| {
                            Cache::new(std::env::var_os("LOCALAPPDATA").map(|p| {
                                PathBuf::from(p).join("FuzeRustPreview/stage-reference-v1.json")
                            }))
                        });
                        cache.get(
                            req.id,
                            req.refresh,
                            now(),
                            &|p| {
                                crate::update_io::get(
                                    "data.aramkit.cn",
                                    p,
                                    LIMIT,
                                    None,
                                    None,
                                    &cancel,
                                )
                            },
                            &cancel,
                        )
                    };
                    if cancel() {
                        continue;
                    }
                    if let Ok(data) = fuze_data::prepare(&roots, req.locale, fuze_data::Scope::Ocr)
                    {
                        reply.names = Arc::new(
                            data.catalog()
                                .iter()
                                .map(|(name, id)| (*id, name.clone()))
                                .collect(),
                        );
                    }
                    let Ok(mut s) = lock.lock() else { return };
                    if !s.stop && s.generation == seen {
                        reply.serial = s.reply.serial.wrapping_add(1);
                        s.reply = reply;
                    }
                }
            })
            .map_err(|_| "stage-worker")?;
        Ok(Self { shared })
    }
    pub fn request(&self, request: Option<Request>) {
        if let Ok(mut s) = self.shared.0.lock() {
            let preserve = request.as_ref().is_some_and(|r| {
                r.refresh
                    && s.desired
                        .as_ref()
                        .is_some_and(|old| old.id == r.id && old.locale == r.locale)
                    && s.reply
                        .snapshot
                        .as_ref()
                        .is_some_and(|p| p.champion == r.id)
            });
            s.desired = request;
            s.generation = s.generation.wrapping_add(1);
            let serial = s.reply.serial.wrapping_add(1);
            if preserve {
                s.reply.serial = serial;
                s.reply.cached = true;
                s.reply.issue = Some("stage-loading");
            } else {
                s.reply = Reply {
                    serial,
                    ..Default::default()
                };
            }
            self.shared.1.notify_one();
        }
    }
    pub fn changed(&self, serial: u64) -> Option<Reply> {
        self.shared
            .0
            .lock()
            .ok()
            .filter(|s| s.reply.serial != serial)
            .map(|s| s.reply.clone())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if let Ok(mut s) = self.shared.0.lock() {
            s.stop = true;
            s.generation = s.generation.wrapping_add(1);
            self.shared.1.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(path: &str) -> Result<Vec<u8>> {
        Ok(if path.ends_with("versions.json"){serde_json::json!({"latest":"16.20","versions":[{"version":"16.20","dataDate":"2026-10-15","dataPath":"data/16.20-20261015-abcdef012345"}]})}else{serde_json::json!({"champion":{"id":21},"augments":{"all":[{"id":1,"winRate":0.6,"pickRate":0.1,"sampleCount":1000}],"stages":{"1":[{"id":2,"winRate":0.8,"pickRate":0.2,"sampleCount":199},{"id":3,"winRate":0.55,"pickRate":0.3,"sampleCount":500}]}}})}.to_string().into_bytes())
    }
    #[test]
    fn refresh_keeps_visible_snapshot_but_context_change_clears_it() {
        let service = Service {
            shared: Arc::new((Mutex::new(State::default()), Condvar::new())),
        };
        let req = Request {
            id: 21,
            locale: fuze_data::Locale::ZhCn,
            refresh: false,
        };
        service.request(Some(req.clone()));
        let snapshot = Arc::new(fetch_snapshot(21, now(), &fixture, &|| false).unwrap());
        service.shared.0.lock().unwrap().reply.snapshot = Some(snapshot.clone());
        service.request(Some(Request {
            refresh: true,
            ..req
        }));
        let state = service.shared.0.lock().unwrap();
        assert!(Arc::ptr_eq(
            state.reply.snapshot.as_ref().unwrap(),
            &snapshot
        ));
        assert_eq!(state.reply.issue, Some("stage-loading"));
        let generation = state.generation;
        drop(state);
        service.request(None);
        let state = service.shared.0.lock().unwrap();
        assert!(state.reply.snapshot.is_none());
        assert_ne!(state.generation, generation);
    }
    #[test]
    fn validates_source_stage_rows_and_preserves_cache_on_failed_refresh() {
        let mut cache = Cache::new(None);
        let at = now();
        let first = cache.get(21, false, at, &fixture, &|| false);
        let s = first.snapshot.unwrap();
        assert_eq!(s.visible(1).len(), 1);
        assert_eq!(s.visible(1)[0].id, 3);
        assert!(s.visible(2).is_empty());
        let cached = cache.get(
            21,
            false,
            at + FRESH + 1,
            &|_| panic!("no automatic refresh"),
            &|| false,
        );
        assert!(cached.cached);
        assert!(cached.snapshot.unwrap().stale(at + FRESH + 1));
        let failed = cache.get(
            21,
            true,
            at + FRESH + 31,
            &|_| Err("fixture-network"),
            &|| false,
        );
        assert!(failed.cached);
        assert_eq!(failed.snapshot.unwrap().champion, 21);
        assert!(cache
            .get(
                21,
                false,
                at + RETAIN + 1,
                &|_| Err("fixture-network"),
                &|| false
            )
            .snapshot
            .is_none());
        assert!(fetch_snapshot(22, at, &fixture, &|| false).is_err());
        assert!(fetch_snapshot(21, at, &fixture, &|| true).is_err());
        assert!(version(&serde_json::json!({"version":"16.20","dataDate":"2026-02-30","dataPath":"data/16.20-20260230-abcdef012345"})).is_err());
        assert!(!valid_rows(&[
            Row {
                id: 1,
                win: 0.5,
                pick: 0.5,
                samples: 1
            },
            Row {
                id: 1,
                win: 0.5,
                pick: 0.5,
                samples: 1
            }
        ]));
    }
    #[test]
    fn disk_cache_reload_downgrade_and_cancellation_preserve_previous_bytes() {
        let root = std::env::temp_dir().join(format!(
            "fuze-stage-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("reference.json");
        let at = now();
        let mut cache = Cache::new(Some(path.clone()));
        assert!(cache
            .get(21, false, at, &fixture, &|| false)
            .snapshot
            .is_some());
        let original = std::fs::read(&path).unwrap();
        let mut reloaded = Cache::new(Some(path.clone()));
        assert!(
            reloaded
                .get(21, false, at, &|_| panic!("cache must not fetch"), &|| {
                    false
                })
                .cached
        );
        let older = |p: &str| {
            fixture(p).map(|bytes| {
                String::from_utf8(bytes)
                    .unwrap()
                    .replace("16.20", "16.19")
                    .into_bytes()
            })
        };
        assert!(reloaded
            .get(21, true, at + 31, &older, &|| false)
            .issue
            .is_some());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let fetch = |p: &str| {
            if !p.ends_with("versions.json") {
                cancelled.store(true, std::sync::atomic::Ordering::Release);
            }
            fixture(p)
        };
        reloaded.get(21, true, at + 400, &fetch, &|| {
            cancelled.load(std::sync::atomic::Ordering::Acquire)
        });
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(json(&vec![b' '; LIMIT + 1]).is_err());
        assert!(
            root.starts_with(std::env::temp_dir())
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("fuze-stage-test-")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
