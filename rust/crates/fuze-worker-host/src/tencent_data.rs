//! On-demand public Mayhem supplement, shared cache; never on UI or OCR threads.
use crate::Result;
use fuze_data::{champion_detail::Detail, recommendations::Recommendation, tencent::Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
const FRESH: u64 = 6 * 3600;
#[derive(Serialize, Deserialize)]
struct Entry {
    fetched: u64,
    bytes: Vec<u8>,
}
#[derive(Default)]
struct Cache {
    entries: HashMap<u32, Entry>,
    retry: HashMap<u32, u64>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
fn path(id: u32) -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join(format!("FuzeRustPreview/tencent101/16.21/{id}.json")))
}
fn date_days(s: &str) -> Option<i64> {
    if s.len() != 8 || !s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut y = s[..4].parse::<i64>().ok()?;
    let m = s[4..6].parse::<i64>().ok()?;
    let d = s[6..].parse::<i64>().ok()?;
    y -= i64::from(m <= 2);
    let era = y / 400;
    let year = y - era * 400;
    Some(
        era * 146097 + year * 365 + year / 4 - year / 100
            + (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5
            + d
            - 1
            - 719468,
    )
}
fn recent(s: &Snapshot, at: u64) -> bool {
    date_days(&s.date).is_some_and(|day| {
        let today = (at / 86400) as i64;
        day >= today - 3 && day <= today + 1
    })
}
impl Cache {
    fn get(
        &mut self,
        id: u32,
        at: u64,
        file: Option<PathBuf>,
        fetch: &dyn Fn() -> Result<Vec<u8>>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Snapshot> {
        if cancel() {
            return Err("tencent-cancelled");
        }
        if !self.entries.contains_key(&id) {
            if let Some(p) = &file {
                if let Ok(bytes) =
                    crate::update_channel::bounded(p, (fuze_data::tencent::LIMIT * 5) as u64)
                {
                    if let Ok(entry) = serde_json::from_slice::<Entry>(&bytes) {
                        self.entries.insert(id, entry);
                    }
                }
            }
        }
        let previous = self
            .entries
            .get(&id)
            .filter(|e| e.fetched <= at && at - e.fetched <= 2 * 86400)
            .and_then(|e| fuze_data::tencent::parse(&e.bytes, id).ok())
            .filter(|s| recent(s, at));
        if previous.is_some()
            && self
                .entries
                .get(&id)
                .is_some_and(|e| at - e.fetched < FRESH)
        {
            return Ok(previous.unwrap());
        }
        if self.retry.get(&id).is_some_and(|t| *t > at) {
            return previous.ok_or("tencent-unavailable");
        }
        self.retry.insert(id, at + 300);
        let result = fetch().and_then(|bytes| {
            let snapshot = fuze_data::tencent::parse(&bytes, id)?;
            if !recent(&snapshot, at) || previous.as_ref().is_some_and(|p| snapshot.date < p.date) {
                return Err("tencent-date");
            }
            if cancel() {
                return Err("tencent-cancelled");
            }
            let entry = Entry { fetched: at, bytes };
            if let Some(file) = &file {
                let _ = crate::preferences::save_json(file, &entry);
            }
            if self.entries.len() >= 16 {
                self.entries.clear();
                self.retry.retain(|_, v| *v > at);
            }
            self.entries.insert(id, entry);
            Ok(snapshot)
        });
        if cancel() {
            return Err("tencent-cancelled");
        }
        result.or_else(|e| previous.ok_or(e))
    }
}
pub fn fetch(id: u32, cancel: &dyn Fn() -> bool) -> Result<Snapshot> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    if id == 0 || id > 10000 {
        return Err("tencent-champion");
    }
    let store = CACHE.get_or_init(|| Mutex::new(Cache::default()));
    let mut cache = loop {
        if cancel() {
            return Err("tencent-cancelled");
        }
        match store.try_lock() {
            Ok(cache) => break cache,
            Err(std::sync::TryLockError::Poisoned(_)) => return Err("tencent-cache"),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
        }
    };
    cache.get(
        id,
        now(),
        path(id),
        &|| {
            crate::update_io::get(
                "mlol.qt.qq.com",
                &format!("{}{id}", fuze_data::tencent::DETAIL_PATH),
                fuze_data::tencent::LIMIT,
                None,
                None,
                cancel,
            )
        },
        cancel,
    )
}
pub struct Supplement {
    pub detail: Detail,
    pub statistics: HashMap<u32, Recommendation>,
}
pub fn load(
    roots: &[fuze_data::Root],
    id: u32,
    identity: &str,
    cancel: &dyn Fn() -> bool,
) -> Result<Supplement> {
    // A fixed item catalogue must never be silently carried into another patch or locale.
    if !identity.starts_with("zh-CN@16.21.") {
        return Err("tencent-version");
    }
    let snapshot = fetch(id, cancel)?;
    if cancel() {
        return Err("tencent-cancelled");
    }
    let mut data = fuze_data::current_augments::prepare_identity(
        roots,
        fuze_data::Locale::ZhCn,
        fuze_data::Scope::Detail(id),
        identity,
    )?;
    data.apply_tencent(&snapshot)?;
    Ok(Supplement {
        detail: data.champion_detail(id)?,
        statistics: data.recommendations(id)?,
    })
}
pub struct Pending {
    receiver: std::sync::mpsc::Receiver<Result<Supplement>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Pending {
    pub fn start(roots: Vec<fuze_data::Root>, id: u32, identity: String) -> Option<Self> {
        if id == 0 || !identity.starts_with("zh-CN@16.21.") {
            return None;
        }
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = stop.clone();
        let (tx, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("fuze-tencent-data".into())
            .spawn(move || {
                let value = load(&roots, id, &identity, &|| {
                    cancel.load(std::sync::atomic::Ordering::Relaxed)
                });
                let _ = tx.send(value);
            })
            .ok()?;
        Some(Self { receiver, stop })
    }
    pub fn poll(&self) -> Option<Result<Supplement>> {
        match self.receiver.try_recv() {
            Ok(value) => Some(value),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("tencent-unavailable")),
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &[u8] = include_bytes!("../../../../tests/fixtures/tencent101/hero-13.json");
    #[test]
    fn finished_worker_never_leaves_a_pending_refresh() {
        let (tx, receiver) = std::sync::mpsc::sync_channel(1);
        let p = Pending {
            receiver,
            stop: Default::default(),
        };
        assert!(p.poll().is_none());
        drop(tx);
        assert!(matches!(p.poll(), Some(Err("tencent-unavailable"))));
    }
    #[test]
    #[ignore = "explicit public HTTP check only; no cache, credentials, game or application launch"]
    fn public_winhttp_read_only() {
        let bytes = crate::update_io::get(
            "mlol.qt.qq.com",
            &format!("{}13", fuze_data::tencent::DETAIL_PATH),
            fuze_data::tencent::LIMIT,
            None,
            None,
            &|| false,
        )
        .unwrap();
        let snapshot = fuze_data::tencent::parse(&bytes, 13).unwrap();
        assert!(!fuze_data::item_sets::create(13, "瑞兹", &[&snapshot.build]).is_empty());
        println!(
            "public WinHTTP Mayhem response: date={}, rankings={}",
            snapshot.date,
            snapshot.ranks.len()
        );
    }
    #[test]
    fn cache_reuse_expiry_failure_and_cancel_use_no_user_directories() {
        let at = (date_days("20261025").unwrap() as u64) * 86400;
        let mut c = Cache::default();
        let first = c
            .get(13, at, None, &|| Ok(FIXTURE.to_vec()), &|| false)
            .unwrap();
        assert_eq!(first.champion, 13);
        assert!(c
            .get(
                13,
                at + 1,
                None,
                &|| panic!("fresh cache must avoid IO"),
                &|| false
            )
            .is_ok());
        assert!(c
            .get(13, at + FRESH, None, &|| Err("offline"), &|| false)
            .is_ok());
        assert!(c
            .get(13, at + 4 * 86400, None, &|| Err("offline"), &|| false)
            .is_err());
        assert!(c
            .get(13, at, None, &|| panic!("cancelled"), &|| true)
            .is_err());
        assert_eq!(date_days("19700101"), Some(0));
        assert!(date_days("20260228").unwrap() < date_days("20260301").unwrap());
    }
    #[test]
    fn real_provider_to_managed_shop_payload_and_readback_preserves_player_sets() {
        let snap = fuze_data::tencent::parse(FIXTURE, 13).unwrap();
        let sets = fuze_data::item_sets::create(13, "瑞兹", &[&snap.build]);
        let user = serde_json::json!({"uid":"player-owned","title":"自己的","blocks":[]});
        let current = serde_json::json!({"accountId":7,"itemSets":[user]});
        let expected = fuze_lcu::item_sets::merge(&current, &sets, 13, 7, 123).unwrap();
        assert_eq!(expected["itemSets"][1], current["itemSets"][0]);
        fuze_lcu::item_sets::verify_saved(&expected, &expected).unwrap();
        let mut wrong = expected.clone();
        wrong["itemSets"][0]["blocks"][0]["items"][0]["id"] = serde_json::json!("999999");
        assert!(fuze_lcu::item_sets::verify_saved(&expected, &wrong).is_err());
    }
    #[test]
    fn public_hero_corpus_produces_only_mapped_mayhem_shop_items() {
        let files: Vec<Vec<u8>> = if let Some(root) = std::env::var_os("FUZE_TENCENT_REVIEW_DIR") {
            std::fs::read_dir(root)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("tencent-hero-")
                })
                .map(|p| std::fs::read(p).unwrap())
                .collect()
        } else {
            vec![FIXTURE.to_vec()]
        };
        let catalog = fuze_data::tencent::catalog();
        let buy: std::collections::HashSet<_> = catalog["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|i| i["purchaseId"].as_u64().map(|id| id.to_string()))
            .collect();
        let mut heroes = 0;
        let mut imported = 0;
        for bytes in files {
            let wrapper: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let raw: serde_json::Value = serde_json::from_str(
                wrapper["data"]["_fieldValues"]
                    .as_object()
                    .unwrap()
                    .values()
                    .next()
                    .unwrap()
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
            let id = raw["augment_json_irank"]
                .as_str()
                .unwrap()
                .split('&')
                .find_map(|s| s.strip_prefix("255:"))
                .unwrap()
                .split('|')
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            let snapshot = fuze_data::tencent::parse(&bytes, id).unwrap();
            let sets = fuze_data::item_sets::create(id, "英雄", &[&snapshot.build]);
            assert!(!sets.is_empty(), "missing build for {id}");
            for set in &sets {
                for block in set["blocks"].as_array().unwrap() {
                    for item in block["items"].as_array().unwrap() {
                        assert!(buy.contains(item["id"].as_str().unwrap()));
                    }
                }
            }
            let expected = fuze_lcu::item_sets::merge(
                &serde_json::json!({"accountId":1,"itemSets":[]}),
                &sets,
                id,
                1,
                1,
            )
            .unwrap();
            fuze_lcu::item_sets::verify_saved(&expected, &expected).unwrap();
            imported += sets.len();
            heroes += 1;
        }
        println!("public Tencent corpus: {heroes} heroes -> {imported} validated Mayhem item sets");
    }
}
