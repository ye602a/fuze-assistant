//! Public statistics reference only. Not a source of actual claims or automatic ranking.
use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
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
    #[serde(default)]
    pub combinations: Option<Vec<Combination>>,
    pub schema: u8,
    pub champion: u32,
    pub patch: String,
    pub date: String,
    pub data_path: String,
    pub fetched: u64,
    pub rows: [Vec<Row>; 5],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Combination {
    pub ids: Vec<u32>,
    pub win: f64,
    pub samples: u64,
}
fn valid_combinations(rows: &[Combination]) -> bool {
    let mut keys = HashSet::new();
    rows.len() <= 512
        && rows.iter().all(|r| {
            let mut ids = r.ids.clone();
            ids.sort_unstable();
            (2..=4).contains(&ids.len())
                && ids.iter().all(|id| *id > 0 && *id <= 100000)
                && ids.windows(2).all(|p| p[0] != p[1])
                && keys.insert(ids)
                && r.samples > 0
                && r.samples <= 9007199254740991
                && r.win.is_finite()
                && (0.0..=1.0).contains(&r.win)
        })
}
fn combinations(value: &Value) -> Option<Vec<Combination>> {
    let rows = value.as_array()?;
    if rows.len() > 512 {
        return None;
    }
    let rows: Vec<_> = rows
        .iter()
        .map(|r| {
            Some(Combination {
                ids: r["augmentIds"]
                    .as_array()?
                    .iter()
                    .map(|id| u32::try_from(id.as_u64()?).ok())
                    .collect::<Option<Vec<_>>>()?,
                win: r["winRate"].as_f64()?,
                samples: r["sampleCount"].as_u64()?,
            })
        })
        .collect::<Option<_>>()?;
    valid_combinations(&rows).then_some(rows)
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
    pub fn usable(&self, champion: u32, at: u64) -> bool {
        self.champion == champion && self.fetched <= at && at - self.fetched <= RETAIN
    }
    pub fn patch_matches(&self, dataset: &str) -> bool {
        let version = dataset.rsplit('@').next().unwrap_or(dataset);
        self.patch == version.split('.').take(2).collect::<Vec<_>>().join(".")
    }
    fn valid(&self, now: u64) -> bool {
        self.combinations.as_ref().is_none_or(|r| valid_combinations(r)) && self.schema==1 && self.champion>0 && self.champion<=10000 && self.fetched<=now && now-self.fetched<=RETAIN && !self.rows[0].is_empty() && self.rows.iter().all(|r|valid_rows(r)) && version(&serde_json::json!({"version":self.patch,"dataDate":self.date,"dataPath":self.data_path})).is_ok()
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
        self.combinations.is_none() || now.saturating_sub(self.fetched) >= FRESH
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
    parse_snapshot(id, now, patch, date, data_path, &data)
}
fn parse_snapshot(
    id: u32,
    now: u64,
    patch: String,
    date: String,
    data_path: String,
    data: &Value,
) -> Result<Snapshot> {
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
        // Missing/invalid combinations and legacy caches remain unavailable. Only an
        // explicit valid array (including []) establishes that records were provided.
        // This must not discard otherwise valid single-augment or stage statistics.
        combinations: combinations(&data["augmentCombinations"]),
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
#[cfg(test)]
pub(crate) fn local_snapshot(
    roots: &[fuze_data::Root],
    locale: fuze_data::Locale,
    id: u32,
    at: u64,
) -> Result<Option<Snapshot>> {
    let data = fuze_data::prepare(roots, locale, fuze_data::Scope::Champion(id))?;
    snapshot_from_prepared(&data, id, at)
}
fn snapshot_from_prepared(
    data: &fuze_data::Prepared,
    id: u32,
    at: u64,
) -> Result<Option<Snapshot>> {
    if data
        .document("manifest.json")
        .is_none_or(|v| v["source"] != fuze_data::aramkit::SOURCE)
    {
        return Ok(None);
    }
    let path = format!("champion-shards/{id}.json");
    let reference = &data.document(&path).ok_or("stage-local-missing")?["champions"]
        [id.to_string()]["aramkitReference"];
    Ok(Some(parse_snapshot(
        id,
        at,
        reference["patch"].as_str().ok_or("stage-format")?.into(),
        reference["date"].as_str().ok_or("stage-format")?.into(),
        reference["dataPath"].as_str().ok_or("stage-format")?.into(),
        reference,
    )?))
}
#[derive(Clone, Default)]
pub struct Reply {
    pub snapshot: Option<Arc<Snapshot>>,
    /// Independent cross-hero catalogue routes; never hero-specific statistics.
    pub routes: Option<Arc<fuze_data::combination_routes::Index>>,
    pub names: Arc<HashMap<u32, String>>,
    pub icons: Arc<HashMap<u32, String>>,
    pub cached: bool,
    pub issue: Option<&'static str>,
    pub serial: u64,
    pub retry_after: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Access {
    CacheOnly,
    Cached,
    Prepare,
    Refresh,
}
#[derive(Clone)]
pub struct Request {
    pub id: u32,
    pub locale: fuze_data::Locale,
    pub access: Access,
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
/// All native reference consumers share one cache/IO lock and one download cooldown.
/// UI peeks never block on IO; entering gameplay revokes pending network work.
pub struct Store {
    cache: Mutex<Option<Cache>>,
    routes: Mutex<Option<RoutesCache>>,
    network: AtomicBool,
    network_epoch: AtomicU64,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            cache: Mutex::new(None),
            routes: Mutex::new(None),
            network: AtomicBool::new(true),
            network_epoch: AtomicU64::new(0),
        }
    }
}
impl Store {
    pub fn allow_network(&self, enabled: bool) {
        if self.network.swap(enabled, Ordering::AcqRel) != enabled {
            self.network_epoch.fetch_add(1, Ordering::AcqRel);
        }
    }
    pub fn peek(&self, id: u32, at: u64) -> Option<Arc<Snapshot>> {
        self.cache
            .try_lock()
            .ok()?
            .as_ref()?
            .entries
            .iter()
            .find(|s| s.usable(id, at))
            .cloned()
    }

    fn routes_for(
        &self,
        roots: &[fuze_data::Root],
        data: &fuze_data::Prepared,
        access: Access,
        cancel: &dyn Fn() -> bool,
    ) -> Option<Arc<fuze_data::combination_routes::Index>> {
        let key = RoutesKey {
            dataset: data.identity(),
            source: data.source(),
            roots: roots.iter().map(|r| (r.path.clone(), r.source)).collect(),
            manifest: data.document("manifest.json")?.clone(),
        };
        let mut cache = self.routes.lock().ok()?;
        cached_routes(
            &mut cache,
            key,
            matches!(access, Access::Prepare | Access::Refresh)
                && self.network.load(Ordering::Acquire),
            cancel,
            || prepare_routes(roots, data, cancel),
        )
    }
}

#[derive(Clone, PartialEq)]
struct RoutesKey {
    dataset: String,
    source: fuze_data::Source,
    roots: Vec<(PathBuf, fuze_data::Source)>,
    manifest: Value,
}
struct RoutesCache {
    key: RoutesKey,
    /// None remembers a missing/unsupported local index for this immutable batch.
    index: Option<Arc<fuze_data::combination_routes::Index>>,
}

fn cached_routes(
    cache: &mut Option<RoutesCache>,
    key: RoutesKey,
    prepare: bool,
    cancel: &dyn Fn() -> bool,
    load: impl FnOnce() -> Option<fuze_data::combination_routes::Index>,
) -> Option<Arc<fuze_data::combination_routes::Index>> {
    if cancel() {
        return None;
    }
    if let Some(old) = cache.as_ref().filter(|old| old.key == key) {
        return old.index.clone();
    }
    if !prepare {
        return None;
    }
    let index = load()
        .filter(|index| index.dataset == key.dataset)
        .map(Arc::new);
    if cancel() {
        return None;
    }
    *cache = Some(RoutesCache {
        key,
        index: index.clone(),
    });
    index
}

/// Reuse the already prepared consumer identity. Only a single matching root can
/// supply Full; incomplete roots cannot silently fall back to another source.
fn prepare_routes(
    roots: &[fuze_data::Root],
    data: &fuze_data::Prepared,
    cancel: &dyn Fn() -> bool,
) -> Option<fuze_data::combination_routes::Index> {
    let manifest = data.document("manifest.json")?;
    if manifest["source"] != fuze_data::aramkit::SOURCE || cancel() {
        return None;
    }
    let identity = data.identity();
    for root in roots.iter().filter(|r| r.source == data.source()) {
        if cancel() {
            return None;
        }
        let root = std::slice::from_ref(root);
        let Ok(probe) = fuze_data::current_augments::prepare_identity(
            root,
            data.locale(),
            data.scope(),
            &identity,
        ) else {
            continue;
        };
        if probe.document("manifest.json") != Some(manifest)
            || probe.document("augments.json") != data.document("augments.json")
        {
            continue;
        }
        if cancel() {
            return None;
        }
        let full = fuze_data::current_augments::prepare_identity(
            root,
            data.locale(),
            fuze_data::Scope::Full,
            &identity,
        )
        .ok()?;
        if cancel()
            || full.source() != data.source()
            || full.document("manifest.json") != Some(manifest)
            || full.document("augments.json") != data.document("augments.json")
        {
            return None;
        }
        return fuze_data::combination_routes::from_prepared(&full);
    }
    None
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
    #[cfg(test)]
    fn get(
        &mut self,
        id: u32,
        refresh: bool,
        at: u64,
        fetch: &dyn Fn(&str) -> Result<Vec<u8>>,
        cancel: &dyn Fn() -> bool,
    ) -> Reply {
        self.get_policy(
            id,
            if refresh {
                Access::Refresh
            } else {
                Access::Cached
            },
            at,
            fetch,
            cancel,
        )
    }
    fn get_policy(
        &mut self,
        id: u32,
        access: Access,
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
        if access == Access::CacheOnly {
            reply.issue = previous.is_none().then_some("stage-cache-missing");
            return reply;
        }
        if previous
            .as_ref()
            .is_some_and(|s| access == Access::Cached || access == Access::Prepare && !s.stale(at))
        {
            return reply;
        }
        if at < self.retry {
            reply.issue = Some("stage-busy");
            reply.retry_after = Some(self.retry);
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
            _ if cancel() => {
                reply.issue = Some("stage-cancelled");
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
        Self::with_store(roots, offline, Arc::new(Store::default()))
    }
    pub fn with_store(
        roots: Vec<fuze_data::Root>,
        offline: bool,
        store: Arc<Store>,
    ) -> Result<Self> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("fuze-stage-reference".into())
            .spawn(move || {
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
                    let prepare_epoch = store.network_epoch.load(Ordering::Acquire);
                    let cancel = || {
                        lock.lock()
                            .map(|s| s.stop || s.generation != seen)
                            .unwrap_or(true)
                    };
                    if cancel() {
                        continue;
                    }
                    // A full ARAMKit update already contains these exact stage/combination rows.
                    // Prefer the local bundle rather than fetching the same hero a second time.
                    let prepared =
                        fuze_data::prepare(&roots, req.locale, fuze_data::Scope::Champion(req.id))
                            .ok();
                    let local = prepared.as_ref().and_then(|data| {
                        snapshot_from_prepared(data, req.id, now()).ok().flatten()
                    });
                    let mut reply = if let Some(snapshot) = local {
                        if cancel() {
                            continue;
                        }
                        let snapshot = Arc::new(snapshot);
                        if let Ok(mut holder) = store.cache.lock() {
                            let cache = holder.get_or_insert_with(|| Cache::new(None));
                            cache.entries.retain(|old| old.champion != req.id);
                            cache.entries.push_back(snapshot.clone());
                            while cache.entries.len() > 8 {
                                cache.entries.pop_front();
                            }
                        }
                        Reply {
                            snapshot: Some(snapshot),
                            cached: true,
                            ..Default::default()
                        }
                    } else if offline {
                        Reply {
                            issue: Some("stage-offline"),
                            ..Default::default()
                        }
                    } else {
                        let Ok(mut holder) = store.cache.lock() else {
                            return;
                        };
                        let epoch = store.network_epoch.load(Ordering::Acquire);
                        let access = if store.network.load(Ordering::Acquire) {
                            req.access
                        } else {
                            Access::CacheOnly
                        };
                        let cancelled = || {
                            cancel()
                                || access != Access::CacheOnly
                                    && (!store.network.load(Ordering::Acquire)
                                        || store.network_epoch.load(Ordering::Acquire) != epoch)
                        };
                        let cache = holder.get_or_insert_with(|| {
                            Cache::new(std::env::var_os("LOCALAPPDATA").map(|p| {
                                PathBuf::from(p).join("FuzeRustPreview/stage-reference-v1.json")
                            }))
                        });
                        cache.get_policy(
                            req.id,
                            access,
                            now(),
                            &|p| {
                                crate::update_io::get(
                                    "data.aramkit.cn",
                                    p,
                                    LIMIT,
                                    None,
                                    None,
                                    &cancelled,
                                )
                            },
                            &cancelled,
                        )
                    };
                    if cancel() {
                        continue;
                    }
                    // The Champion preparation already includes the OCR catalogue.
                    // Reuse its identity and source for both names and optional routes.
                    let prepared = prepared.or_else(|| {
                        fuze_data::prepare(&roots, req.locale, fuze_data::Scope::Ocr).ok()
                    });
                    if let Some(data) = prepared {
                        reply.names = Arc::new(
                            data.catalog()
                                .iter()
                                .map(|(name, id)| (*id, name.clone()))
                                .collect(),
                        );
                        reply.icons = Arc::new(
                            data.recognition_cards()
                                .into_iter()
                                .filter(|card| !card.icon.is_empty())
                                .map(|card| (card.id, card.icon))
                                .collect(),
                        );
                        let preparing = matches!(req.access, Access::Prepare | Access::Refresh);
                        let routes_cancelled = || {
                            cancel()
                                || preparing
                                    && (!store.network.load(Ordering::Acquire)
                                        || store.network_epoch.load(Ordering::Acquire)
                                            != prepare_epoch)
                        };
                        reply.routes =
                            store.routes_for(&roots, &data, req.access, &routes_cancelled);
                        if routes_cancelled() {
                            reply.routes = None;
                        }
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
                r.access == Access::Refresh
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

    fn routes_key() -> RoutesKey {
        RoutesKey {
            dataset: "zh-CN@16.22.1".into(),
            source: fuze_data::Source::Cache,
            roots: vec![(PathBuf::from("fixture"), fuze_data::Source::Cache)],
            manifest: serde_json::json!({"source":"ARAMKit","dataVersion":"16.22.1"}),
        }
    }
    fn routes_index() -> fuze_data::combination_routes::Index {
        fuze_data::combination_routes::Index {
            dataset: "zh-CN@16.22.1".into(),
            patch: "16.22".into(),
            date: "2026-11-12".into(),
            data_path: "data/16.22-20261112-bfae7c1ce948".into(),
            rows: vec![fuze_data::combination_routes::Route {
                members: vec![1004, 3086],
                champions: vec![17, 145],
            }],
            champions: 2,
        }
    }
    #[test]
    fn route_cache_reuses_one_identity_and_remembers_absent_data_without_gameplay_scan() {
        let mut cache = None;
        let key = routes_key();
        assert!(
            cached_routes(&mut cache, key.clone(), false, &|| false, || {
                panic!("CacheOnly must not load Full")
            })
            .is_none()
        );
        let first = cached_routes(&mut cache, key.clone(), true, &|| false, || {
            Some(routes_index())
        })
        .unwrap();
        for prepare in [false, true] {
            let next = cached_routes(&mut cache, key.clone(), prepare, &|| false, || {
                panic!("same identity must not rescan")
            })
            .unwrap();
            assert!(Arc::ptr_eq(&first, &next));
        }
        for change in 0..4 {
            let mut changed = key.clone();
            match change {
                0 => changed.dataset = "zh-CN@16.22.2".into(),
                1 => changed.source = fuze_data::Source::Bundled,
                2 => changed.roots[0].0 = PathBuf::from("other-root"),
                _ => changed.manifest["dataDate"] = serde_json::json!("2026-11-13"),
            }
            assert!(cached_routes(&mut cache, changed, false, &|| false, || {
                panic!("a new gameplay context must not build or borrow old routes")
            })
            .is_none());
        }
        let mut missing = key.clone();
        missing.dataset = "zh-CN@16.22.2".into();
        assert!(cached_routes(&mut cache, missing.clone(), true, &|| false, || None).is_none());
        assert!(cached_routes(&mut cache, missing, true, &|| false, || {
            panic!("unsupported batch must be remembered")
        })
        .is_none());
    }

    #[test]
    fn cancelled_route_build_never_publishes_or_replaces_previous_identity() {
        let mut cache = None;
        let key = routes_key();
        let previous = cached_routes(&mut cache, key.clone(), true, &|| false, || {
            Some(routes_index())
        })
        .unwrap();
        let cancelled = AtomicBool::new(false);
        let mut next = key.clone();
        next.dataset = "zh-CN@16.22.2".into();
        let result = cached_routes(
            &mut cache,
            next.clone(),
            true,
            &|| cancelled.load(Ordering::Acquire),
            || {
                cancelled.store(true, Ordering::Release);
                let mut index = routes_index();
                index.dataset = next.dataset.clone();
                Some(index)
            },
        );
        assert!(result.is_none());
        assert!(cache
            .as_ref()
            .is_some_and(|c| c.key == key && Arc::ptr_eq(c.index.as_ref().unwrap(), &previous)));
        assert!(cached_routes(&mut cache, key, true, &|| true, || {
            panic!("cancelled preparation must not start")
        })
        .is_none());
    }

    struct RoutesBundle(PathBuf);
    impl RoutesBundle {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "fuze-stage-routes-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let bundle = Self(root);
            let mut documents = std::collections::BTreeMap::new();
            documents.insert(
                "augments.json".to_string(),
                serde_json::json!({"data":[
                    {"id":1004,"name":"珠光护手","rarity":1},
                    {"id":3086,"name":"蛋白粉奶昔","rarity":1}
                ]}),
            );
            documents.insert("champions.json".into(), serde_json::json!({"data":[]}));
            documents.insert("items.json".into(), serde_json::json!({"data":[]}));
            documents.insert(
                "champion-shards/index.json".into(),
                serde_json::json!({"shards":[
                    {"path":"champion-shards/17.json","championIds":[17]},
                    {"path":"champion-shards/145.json","championIds":[145]}
                ]}),
            );
            for id in [17, 145] {
                documents.insert(format!("champion-shards/{id}.json"), serde_json::json!({
                    "champions":{id.to_string():{
                        "champion":{"id":id},
                        "aramkitReference":{
                            "patch":"16.22","date":"2026-11-12",
                            "dataPath":"data/16.22-20261112-bfae7c1ce948",
                            "augments":{"all":[{"id":1004,"winRate":0.52,"pickRate":0.1,"sampleCount":1000}],"stages":{}},
                            "augmentCombinations":[{"augmentIds":[1004,3086],"winRate":0.64,"sampleCount":1000}]
                        }
                    }}
                }));
            }
            let files: Vec<_> = documents
                .keys()
                .map(|path| serde_json::json!({"path":path}))
                .collect();
            documents.insert(
                "manifest.json".into(),
                serde_json::json!({
                    "source":"ARAMKit","locale":"zh-CN","dataVersion":"16.22.1",
                    "gamePatch":"16.22","dataDate":"2026-11-12",
                    "dataPath":"data/16.22-20261112-bfae7c1ce948","files":files
                }),
            );
            for (path, value) in documents {
                let path = bundle.0.join("versions/16.22.1").join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            std::fs::write(
                bundle.0.join("current.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schemaVersion":3,"dataVersion":"16.22.1","locale":"zh-CN"
                }))
                .unwrap(),
            )
            .unwrap();
            bundle
        }
        fn root(&self) -> fuze_data::Root {
            fuze_data::Root {
                path: self.0.clone(),
                source: fuze_data::Source::Cache,
            }
        }
    }
    impl Drop for RoutesBundle {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn local_route_preparation_is_optional_and_cache_only_reuses_it_after_gameplay_starts() {
        let bundle = RoutesBundle::new();
        let roots = vec![bundle.root()];
        let data = fuze_data::prepare(
            &roots,
            fuze_data::Locale::ZhCn,
            fuze_data::Scope::Champion(145),
        )
        .unwrap();
        let store = Store::default();
        let hero = snapshot_from_prepared(&data, 145, now()).unwrap().unwrap();
        assert_eq!(hero.combinations.as_ref().unwrap()[0].win, 0.64);
        assert!(store
            .routes_for(&roots, &data, Access::CacheOnly, &|| false)
            .is_none());
        let routes = store
            .routes_for(&roots, &data, Access::Prepare, &|| false)
            .unwrap();
        assert_eq!(routes.rows[0].champions, [17, 145]);
        // A Full re-read would now fail; the already built identity remains usable.
        std::fs::remove_file(bundle.0.join("versions/16.22.1/champion-shards/17.json")).unwrap();
        store.allow_network(false);
        let cached = store
            .routes_for(&roots, &data, Access::CacheOnly, &|| false)
            .unwrap();
        assert!(Arc::ptr_eq(&routes, &cached));
        assert_eq!(
            snapshot_from_prepared(&data, 145, now())
                .unwrap()
                .unwrap()
                .rows[0][0]
                .win,
            0.52
        );
        let untouched = Store::default();
        untouched.allow_network(false);
        assert!(untouched
            .routes_for(&roots, &data, Access::Prepare, &|| false)
            .is_none());
    }

    #[test]
    fn full_routes_do_not_fall_back_to_a_different_root_than_the_matching_local_bundle() {
        let first = RoutesBundle::new();
        let second = RoutesBundle::new();
        std::fs::remove_file(first.0.join("versions/16.22.1/champion-shards/17.json")).unwrap();
        let roots = vec![first.root(), second.root()];
        let data = fuze_data::prepare(
            &roots,
            fuze_data::Locale::ZhCn,
            fuze_data::Scope::Champion(145),
        )
        .unwrap();
        assert!(snapshot_from_prepared(&data, 145, now()).unwrap().is_some());
        assert!(prepare_routes(&roots, &data, &|| false).is_none());
        assert!(fuze_data::current_augments::prepare_identity(
            &roots,
            fuze_data::Locale::ZhCn,
            fuze_data::Scope::Full,
            &data.identity()
        )
        .is_ok());
    }

    #[test]
    fn same_download_parses_combinations_and_legacy_cache_remains_readable() {
        let payload = serde_json::json!([
            {"rank":1,"augmentIds":[1004,3086],"sampleCount":6819,"pickRate":0.0102,"winRate":0.6495}
        ]);
        let fetch = |path: &str| {
            let mut value: Value = serde_json::from_slice(&fixture(path)?).unwrap();
            if !path.ends_with("versions.json") {
                value["augmentCombinations"] = payload.clone();
            }
            Ok(serde_json::to_vec(&value).unwrap())
        };
        let s = fetch_snapshot(21, 100, &fetch, &|| false).unwrap();
        let rows = s.combinations.as_ref().unwrap();
        assert_eq!(rows[0].ids, [1004, 3086]);
        assert_eq!(rows[0].samples, 6819);
        assert_eq!(rows[0].win, 0.6495);
        assert_eq!(s.rows[0][0].win, 0.6);
        assert!(!s.stale(101));
        let mut old = serde_json::to_value(&s).unwrap();
        old.as_object_mut().unwrap().remove("combinations");
        let legacy: Snapshot = serde_json::from_value(old).unwrap();
        assert!(legacy.valid(101) && legacy.usable(21, 101));
        assert!(legacy.combinations.is_none() && legacy.stale(101));
        let mut cache = Cache::new(None);
        cache.get_policy(21, Access::Prepare, 100, &fetch, &|| false);
        let reply = cache.get_policy(
            21,
            Access::CacheOnly,
            101,
            &|_| panic!("in-game network"),
            &|| false,
        );
        assert_eq!(
            reply.snapshot.unwrap().combinations.as_ref().unwrap()[0].samples,
            6819
        );
    }

    #[test]
    fn invalid_or_missing_combinations_do_not_hide_single_augment_statistics() {
        for payload in [
            serde_json::json!(null),
            serde_json::json!([{"augmentIds":[1,1],"sampleCount":20,"winRate":0.5}]),
            serde_json::json!([{"augmentIds":[1,2],"sampleCount":0,"winRate":0.5}]),
            serde_json::json!([{"augmentIds":[1,2],"sampleCount":20,"winRate":50}]),
            serde_json::json!([{"augmentIds":[1,2],"sampleCount":20,"winRate":0.5}, {"augmentIds":[2,1],"sampleCount":20,"winRate":0.5}]),
        ] {
            let fetch = |path: &str| {
                let mut value: Value = serde_json::from_slice(&fixture(path)?).unwrap();
                if !path.ends_with("versions.json") {
                    value["augmentCombinations"] = payload.clone();
                }
                Ok(serde_json::to_vec(&value).unwrap())
            };
            let s = fetch_snapshot(21, 100, &fetch, &|| false).unwrap();
            assert!(s.combinations.is_none());
            assert!(s.stale(101));
            assert_eq!(s.rows[0][0].win, 0.6);
            assert_eq!(s.rows[1][0].samples, 199);
        }
    }
    #[test]
    fn explicit_empty_combinations_are_distinct_from_absent_provider_data() {
        let empty = fetch_snapshot(21, 100, &fixture, &|| false).unwrap();
        assert!(empty.combinations.as_ref().unwrap().is_empty());
        assert!(!empty.stale(101));
        let fetch = |path: &str| {
            let mut value: Value = serde_json::from_slice(&fixture(path)?).unwrap();
            if !path.ends_with("versions.json") {
                value.as_object_mut().unwrap().remove("augmentCombinations");
            }
            Ok(serde_json::to_vec(&value).unwrap())
        };
        let absent = fetch_snapshot(21, 100, &fetch, &|| false).unwrap();
        assert!(absent.combinations.is_none() && absent.stale(101));
        assert!(absent.usable(21, 101));
        assert_eq!(absent.rows[0][0].samples, empty.rows[0][0].samples);
        assert_eq!(absent.rows[1][0].samples, empty.rows[1][0].samples);
        let roundtrip: Snapshot =
            serde_json::from_slice(&serde_json::to_vec(&absent).unwrap()).unwrap();
        assert!(roundtrip.valid(101) && roundtrip.combinations.is_none());
    }
    #[test]
    fn prepare_reuses_fresh_cache_and_gameplay_never_fetches_or_promotes_missing_rows() {
        let at = now();
        let mut cache = Cache::new(None);
        assert_eq!(
            cache
                .get_policy(
                    21,
                    Access::CacheOnly,
                    at,
                    &|_| panic!("gameplay network"),
                    &|| false
                )
                .issue,
            Some("stage-cache-missing")
        );
        assert!(cache
            .get_policy(21, Access::Prepare, at, &fixture, &|| false)
            .snapshot
            .is_some());
        assert!(
            cache
                .get_policy(
                    21,
                    Access::Prepare,
                    at + 31,
                    &|_| panic!("fresh cache redownload"),
                    &|| false
                )
                .cached
        );
        assert!(
            cache
                .get_policy(
                    21,
                    Access::CacheOnly,
                    at + FRESH + 1,
                    &|_| panic!("gameplay refresh"),
                    &|| false
                )
                .cached
        );
        assert!(cache
            .get_policy(
                21,
                Access::CacheOnly,
                at + RETAIN + 1,
                &|_| panic!("expired gameplay fetch"),
                &|| false
            )
            .snapshot
            .is_none());
        assert!(
            !cache
                .get_policy(21, Access::Prepare, at + FRESH + 1, &fixture, &|| false)
                .cached
        );
    }
    #[test]
    fn shared_cache_peek_never_waits_for_io_and_epoch_changes_invalidate_network() {
        let store = Store::default();
        let initial = store.network_epoch.load(Ordering::Acquire);
        store.allow_network(true);
        assert_eq!(store.network_epoch.load(Ordering::Acquire), initial);
        store.allow_network(false);
        store.allow_network(true);
        assert_ne!(store.network_epoch.load(Ordering::Acquire), initial);
        let mut held = store.cache.lock().unwrap();
        let mut cache = Cache::new(None);
        let at = now();
        cache.get_policy(21, Access::Prepare, at, &fixture, &|| false);
        *held = Some(cache);
        assert!(store.peek(21, at).is_none());
        drop(held);
        assert_eq!(store.peek(21, at).unwrap().champion, 21);
        assert!(store.peek(22, at).is_none());
    }
    fn fixture(path: &str) -> Result<Vec<u8>> {
        Ok(if path.ends_with("versions.json"){serde_json::json!({"latest":"16.20","versions":[{"version":"16.20","dataDate":"2026-10-15","dataPath":"data/16.20-20261015-abcdef012345"}]})}else{serde_json::json!({"champion":{"id":21},"augmentCombinations":[],"augments":{"all":[{"id":1,"winRate":0.6,"pickRate":0.1,"sampleCount":1000}],"stages":{"1":[{"id":2,"winRate":0.8,"pickRate":0.2,"sampleCount":199},{"id":3,"winRate":0.55,"pickRate":0.3,"sampleCount":500}]}}})}.to_string().into_bytes())
    }
    #[test]
    fn refresh_keeps_visible_snapshot_but_context_change_clears_it() {
        let service = Service {
            shared: Arc::new((Mutex::new(State::default()), Condvar::new())),
        };
        let req = Request {
            id: 21,
            locale: fuze_data::Locale::ZhCn,
            access: Access::Cached,
        };
        service.request(Some(req.clone()));
        let snapshot = Arc::new(fetch_snapshot(21, now(), &fixture, &|| false).unwrap());
        service.shared.0.lock().unwrap().reply.snapshot = Some(snapshot.clone());
        service.request(Some(Request {
            access: Access::Refresh,
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
