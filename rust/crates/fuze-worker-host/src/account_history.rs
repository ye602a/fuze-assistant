//! Manual, bounded history IO. No refresh timers and no OCR-thread work.
use crate::Result;
use fuze_lcu::history::{Match, Owner};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub fn error_label(error: Option<&str>) -> &'static str {
    match error {
        None => "",
        Some("history-platform-unknown" | "history-platform-conflict") => {
            "区服未确认，请确认客户端已登录"
        }
        Some("history-account-changed" | "history-owner-mismatch") => "账号发生变化，请重新查询",
        Some("history-idle-required") => "返回大厅后可查询战绩",
        Some(
            "history-client-unavailable" | "history-discovery-failed" | "history-connect-failed",
        ) => "客户端连接未就绪",
        Some("history-cache-invalid") => "本地缓存无法读取，原文件已保留",
        Some(_) => "查询未完成；已保留本地缓存，可稍后重试",
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    accounts: Vec<Account>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    key: String,
    platform: String,
    rows: Vec<Match>,
    #[serde(default)]
    observations: Vec<fuze_lcu::history::Observation>,
}
#[derive(Clone, Debug, Default)]
pub struct View {
    pub augment_names: std::sync::Arc<std::collections::HashMap<u32, String>>,
    pub observations: std::sync::Arc<Vec<fuze_lcu::history::Observation>>,
    pub details: std::sync::Arc<Vec<std::sync::Arc<fuze_data::champion_detail::Detail>>>,
    pub asset_version: String,
    pub journal_preview: bool,
    pub account: String,
    pub platform: String,
    pub rows: std::sync::Arc<Vec<Match>>,
    pub error: Option<&'static str>,
    pub cached: bool,
    pub pending: usize,
    pub next_start: Option<u32>,
    pub source: String,
}
// UI snapshots share immutable vectors: comparing hundreds of records every timer tick
// adds no information. New snapshots invalidate once, even when their contents match.
impl PartialEq for View {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.rows, &other.rows)
            && std::sync::Arc::ptr_eq(&self.details, &other.details)
            && std::sync::Arc::ptr_eq(&self.observations, &other.observations)
            && std::sync::Arc::ptr_eq(&self.augment_names, &other.augment_names)
            && self.asset_version == other.asset_version
            && self.journal_preview == other.journal_preview
            && self.account == other.account
            && self.platform == other.platform
            && self.error == other.error
            && self.cached == other.cached
            && self.pending == other.pending
            && self.next_start == other.next_start
            && self.source == other.source
    }
}
pub struct Import {
    owner: Owner,
    rows: Vec<Match>,
    observation: Option<fuze_lcu::history::Observation>,
}
fn key(owner: &Owner) -> Result<String> {
    let mut material = owner.cache_material();
    let hash = crate::update_io::digest(&material, false);
    for b in &mut material {
        unsafe {
            std::ptr::write_volatile(b, 0);
        }
    }
    Ok(crate::update_io::hex(&hash?))
}
pub fn path() -> Result<PathBuf> {
    Ok(crate::preferences::path()?.with_file_name("account-history-v3.json"))
}
/// Explicit offline browsing; never claims this is the currently logged-in account.
pub fn offline(path: &Path, index: usize) -> Result<View> {
    let saved = read(path)?;
    if saved.accounts.is_empty() {
        return Ok(View::default());
    }
    let a = &saved.accounts[index % saved.accounts.len()];
    Ok(View {
        account: a.key[..8].into(),
        platform: a.platform.clone(),
        rows: a.rows.clone().into(),
        observations: a.observations.clone().into(),
        cached: true,
        ..Default::default()
    })
}
fn read(path: &Path) -> Result<Saved> {
    // New fields never rewrite an older binary's strict schema. Read old data only
    // until the first successful save; malformed new schemas never silently fall back.
    let legacy;
    let path = if !path.exists()
        && path
            .file_name()
            .is_some_and(|n| n == "account-history-v2.json" || n == "account-history-v3.json")
    {
        let v2 = path.with_file_name("account-history-v2.json");
        legacy = if path
            .file_name()
            .is_some_and(|n| n == "account-history-v3.json")
            && v2.exists()
        {
            v2
        } else {
            path.with_file_name("account-history.json")
        };
        legacy.as_path()
    } else {
        path
    };
    if !path.exists() {
        return Ok(Saved::default());
    }
    let mut data: Saved =
        serde_json::from_slice(&crate::update_channel::bounded(path, 6 * 1024 * 1024)?)
            .map_err(|_| "history-cache-invalid")?;
    let mut keys = std::collections::HashSet::new();
    if data.accounts.len() > 5
        || data.accounts.iter().any(|a| {
            !keys.insert(&a.key)
                || a.key.len() != 64
                || !a.key.bytes().all(|b| b.is_ascii_hexdigit())
                || fuze_lcu::history::platform(&serde_json::json!(a.platform)).as_deref()
                    != Some(&a.platform)
                || a.rows.len() > 500
                || a.rows.iter().any(|m| !m.valid())
                || a.observations.len() > 100
                || a.observations.iter().any(|o| !o.valid())
        })
    {
        return Err("history-cache-invalid");
    }
    if path.file_name().is_some_and(|n| {
        n == "account-history-v3.json"
            || n == "account-history-v2.json"
            || n == "account-history.json"
    }) {
        let new_journal = path.with_file_name("observations-v2.json");
        let journal = if new_journal.exists() {
            new_journal
        } else {
            path.with_file_name("observations.json")
        };
        for account in &mut data.accounts {
            if let Ok(records) =
                crate::local_records::bound_observations(&journal, &account.key, &account.platform)
            {
                for record in records {
                    if let Some(old) = account
                        .observations
                        .iter_mut()
                        .find(|o| o.game_id == record.game_id)
                    {
                        if record.events.len() > old.events.len() {
                            *old = record;
                        }
                    } else {
                        account.observations.push(record);
                    }
                }
                account
                    .observations
                    .sort_by_key(|o| std::cmp::Reverse(o.game_id));
                account.observations.truncate(100);
            }
        }
    }
    Ok(data)
}
fn cached(path: &Path, owner: &Owner) -> Result<Vec<Match>> {
    let key = key(owner)?;
    Ok(read(path)?
        .accounts
        .into_iter()
        .find(|a| a.key == key)
        .map(|a| a.rows)
        .unwrap_or_default())
}
fn merge(path: &Path, owner: &Owner, rows: Vec<Match>, importing: bool) -> Result<Vec<Match>> {
    if rows.len() > 500 || rows.iter().any(|r| !r.valid()) {
        return Err("history-invalid-response");
    }
    let key = key(owner)?;
    let mut saved = read(path)?;
    let previous = saved
        .accounts
        .iter()
        .position(|a| a.key == key)
        .map(|i| saved.accounts.remove(i));
    let (old, observations) = previous
        .map(|a| (a.rows, a.observations))
        .unwrap_or_default();
    let (first, second) = if importing { (old, rows) } else { (rows, old) };
    let mut ids = std::collections::HashSet::new();
    let mut merged: Vec<_> = first
        .into_iter()
        .chain(second)
        .filter(|r| ids.insert(r.game_id))
        .collect();
    merged.sort_by_key(|r| std::cmp::Reverse((r.created, r.game_id)));
    merged.truncate(500);
    saved.accounts.insert(
        0,
        Account {
            key,
            platform: owner.platform().into(),
            rows: merged.clone(),
            observations,
        },
    );
    saved.accounts.truncate(5);
    crate::preferences::save_json(path, &saved)?;
    Ok(merged)
}
trait Source {
    fn owner(&self) -> Result<Owner>;
    fn rows(&self, owner: &Owner) -> Result<Vec<Match>>;
}
impl Source for fuze_lcu::discovery::HistorySession {
    fn owner(&self) -> Result<Owner> {
        self.owner()
    }
    fn rows(&self, owner: &Owner) -> Result<Vec<Match>> {
        self.rows(owner)
    }
}
pub fn refresh(manual: Option<&Path>, path: &Path) -> View {
    refresh_page(manual, path, 0, None, None)
}
pub fn refresh_page(
    manual: Option<&Path>,
    path: &Path,
    start: u32,
    expected: Option<(&str, &str)>,
    source: Option<&str>,
) -> View {
    struct Page {
        session: fuze_lcu::discovery::HistorySession,
        owner: Owner,
        start: u32,
        count: std::cell::Cell<usize>,
        source: std::cell::Cell<&'static str>,
    }
    impl Source for Page {
        fn owner(&self) -> Result<Owner> {
            let owner = self.session.owner()?;
            if owner != self.owner {
                return Err("history-account-changed");
            }
            Ok(owner)
        }
        fn rows(&self, owner: &Owner) -> Result<Vec<Match>> {
            let rows = match self.source.get() {
                "lcu" => self.session.rows_page(owner, self.start)?,
                "sgp" => self.session.sgp_rows(owner, self.start)?,
                _ => match self.session.sgp_rows(owner, self.start) {
                    Ok(rows) => {
                        self.source.set("sgp");
                        rows
                    }
                    Err(
                        e @ ("history-account-changed"
                        | "history-owner-mismatch"
                        | "history-platform-conflict"
                        | "history-idle-required"),
                    ) => return Err(e),
                    Err(_) => {
                        let rows = self.session.rows_page(owner, self.start)?;
                        self.source.set("lcu");
                        rows
                    }
                },
            };
            self.count.set(rows.len());
            Ok(rows)
        }
    }
    let prepared = (|| {
        if start > 950 || start % 50 != 0 {
            return Err("history-page-limit");
        }
        let session = fuze_lcu::discovery::HistorySession::open(manual)?;
        let owner = session.owner()?;
        if let Some((account, platform)) = expected {
            if &key(&owner)?[..8] != account || owner.platform() != platform {
                return Err("history-account-changed");
            }
        }
        Ok(Page {
            session,
            owner,
            start,
            count: std::cell::Cell::new(0),
            source: std::cell::Cell::new(match source {
                Some("lcu") => "lcu",
                Some("sgp") => "sgp",
                _ => "",
            }),
        })
    })();
    match prepared {
        Ok(page) => {
            let mut view = refresh_from(&page, path);
            view.source = page.source.get().into();
            view.next_start = if view.error.is_some() {
                Some(start)
            } else if (page.count.get() > 0 || page.source.get() == "lcu") && start < 950 {
                Some(start + 50)
            } else {
                None
            };
            view
        }
        Err(error) => View {
            error: Some(error),
            ..Default::default()
        },
    }
}
fn refresh_from(session: &impl Source, path: &Path) -> View {
    let mut view = View::default();
    let result = (|| {
        let owner = session.owner()?;
        view.platform = owner.platform().into();
        view.account = key(&owner)?[..8].into();
        let result = session.rows(&owner);
        if session.owner()? != owner {
            return Err("history-account-changed");
        }
        let owner_key = key(&owner)?;
        view.observations = read(path)?
            .accounts
            .into_iter()
            .find(|a| a.key == owner_key)
            .map(|a| a.observations)
            .unwrap_or_default()
            .into();
        match result {
            Ok(rows) => {
                view.rows = merge(path, &owner, rows, false)?.into();
            }
            Err(error) => {
                // Owner was verified after the request above, including on failures.
                view.rows = cached(path, &owner)?.into();
                view.cached = true;
                return Err(error);
            }
        }
        Ok(())
    })();
    view.error = result.err();
    view
}
pub fn preview(manual: Option<&Path>, source: &Path) -> Result<Import> {
    let session = fuze_lcu::discovery::HistorySession::open(manual)?;
    let owner = session.owner()?;
    let bytes = crate::update_channel::bounded(source, 6 * 1024 * 1024)?;
    let value = serde_json::from_slice(&bytes).map_err(|_| "history-legacy-schema")?;
    let rows = fuze_lcu::history::legacy(&owner, &value)?;
    if rows.is_empty() {
        return Err("history-import-empty");
    }
    if session.owner()? != owner {
        return Err("history-account-changed");
    }
    Ok(Import {
        owner,
        rows,
        observation: None,
    })
}
pub fn preview_journal(manual: Option<&Path>, source: &Path, game: u64) -> Result<Import> {
    let session = fuze_lcu::discovery::HistorySession::open(manual)?;
    let owner = session.owner()?;
    let bytes = crate::update_channel::bounded(source, 4 * 1024 * 1024)?;
    let value = serde_json::from_slice(&bytes).map_err(|_| "journal-schema")?;
    let observation = fuze_lcu::history::legacy_observations(&owner, &value, game)?;
    if session.owner()? != owner {
        return Err("history-account-changed");
    }
    Ok(Import {
        owner,
        rows: vec![],
        observation: Some(observation),
    })
}
impl Import {
    /// Select only reviewed rows from the same preview; never accept arbitrary match IDs.
    pub fn select(mut self, games: &[u64]) -> Result<Self> {
        if self.observation.is_some() {
            return if games.is_empty() {
                Ok(self)
            } else {
                Err("history-import-selection")
            };
        }
        let selected: std::collections::HashSet<_> = games.iter().copied().collect();
        if selected.is_empty()
            || selected.len() != games.len()
            || selected
                .iter()
                .any(|id| !self.rows.iter().any(|r| r.game_id == *id))
        {
            return Err("history-import-selection");
        }
        self.rows.retain(|r| selected.contains(&r.game_id));
        Ok(self)
    }
    pub fn view(&self) -> View {
        View {
            rows: self.rows.clone().into(),
            account: key(&self.owner)
                .map(|k| k[..8].to_owned())
                .unwrap_or_default(),
            platform: self.owner.platform().into(),
            pending: if self.observation.is_some() {
                1
            } else {
                self.rows.len()
            },
            journal_preview: self.observation.is_some(),
            observations: self
                .observation
                .clone()
                .into_iter()
                .collect::<Vec<_>>()
                .into(),
            ..View::default()
        }
    }
    pub fn confirm(self, manual: Option<&Path>, path: &Path) -> Result<View> {
        let session = fuze_lcu::discovery::HistorySession::open(manual)?;
        if session.owner()? != self.owner {
            return Err("history-account-changed");
        }
        if let Some(observation) = self.observation {
            return save_observation(path, &self.owner, observation);
        }
        let rows = merge(path, &self.owner, self.rows, true)?;
        let owner_key = key(&self.owner)?;
        let observations = read(path)?
            .accounts
            .into_iter()
            .find(|a| a.key == owner_key)
            .map(|a| a.observations)
            .unwrap_or_default()
            .into();
        Ok(View {
            observations,
            account: key(&self.owner)?[..8].into(),
            platform: self.owner.platform().into(),
            rows: rows.into(),
            cached: true,
            ..View::default()
        })
    }
}
fn save_observation(
    path: &Path,
    owner: &Owner,
    observation: fuze_lcu::history::Observation,
) -> Result<View> {
    if !observation.valid() {
        return Err("journal-schema");
    }
    let key = key(owner)?;
    let mut saved = read(path)?;
    let index = saved
        .accounts
        .iter()
        .position(|a| a.key == key)
        .ok_or("journal-history-required")?;
    let account = &mut saved.accounts[index];
    if !account
        .rows
        .iter()
        .any(|r| r.game_id == observation.game_id)
    {
        return Err("journal-history-required");
    }
    if !account
        .observations
        .iter()
        .any(|o| o.game_id == observation.game_id)
    {
        account.observations.insert(0, observation);
        account.observations.truncate(100);
        crate::preferences::save_json(path, &saved)?;
    }
    offline(path, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selective_preview_never_adds_unreviewed_matches_or_claims() {
        let make = || Import {
            owner: owner("fixture-account", "HN1"),
            rows: vec![row(1), row(2)],
            observation: None,
        };
        assert_eq!(make().view().rows.len(), 2);
        assert!(make().select(&[]).is_err());
        assert!(make().select(&[1, 1]).is_err());
        assert!(make().select(&[3]).is_err());
        let chosen = make().select(&[2]).unwrap().view();
        assert_eq!(chosen.pending, 1);
        assert_eq!(chosen.rows[0].game_id, 2);
        assert!(chosen.observations.is_empty());
    }
    #[test]
    fn new_cache_keeps_legacy_bytes_and_rejects_corrupt_new_schema() {
        let root = std::env::temp_dir().join(format!("fuze-history-v2-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let old = root.join("account-history.json");
        let new = root.join("account-history-v2.json");
        let account = owner("fixture-account", "HN1");
        merge(&old, &account, vec![row(1)], false).unwrap();
        let bytes = std::fs::read(&old).unwrap();
        assert_eq!(read(&new).unwrap().accounts[0].rows.len(), 1);
        merge(&new, &account, vec![row(2)], false).unwrap();
        assert_eq!(read(&new).unwrap().accounts[0].rows.len(), 2);
        assert_eq!(std::fs::read(&old).unwrap(), bytes);
        let v3 = root.join("account-history-v3.json");
        let v2_bytes = std::fs::read(&new).unwrap();
        assert_eq!(read(&v3).unwrap().accounts[0].rows.len(), 2);
        merge(&v3, &account, vec![row(3)], false).unwrap();
        assert_eq!(std::fs::read(&new).unwrap(), v2_bytes);
        std::fs::write(&v3, b"invalid").unwrap();
        assert!(matches!(read(&v3), Err("history-cache-invalid")));
        std::fs::remove_file(v3).unwrap();
        std::fs::write(&new, b"invalid").unwrap();
        assert!(matches!(read(&new), Err("history-cache-invalid")));
        std::fs::remove_file(old).unwrap();
        std::fs::remove_file(new).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn journal_is_account_bound_idempotent_and_survives_history_refresh() {
        let dir = std::env::temp_dir().join(format!("fuze-journal-fixture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.json");
        let a = owner("fixture-account-A", "HN1");
        let b = owner("fixture-account-B", "HN1");
        merge(&path, &a, vec![row(1)], false).unwrap();
        let observation = fuze_lcu::history::Observation {
            recognition: None,
            game_id: 1,
            interrupted: true,
            capped: false,
            events: vec![fuze_lcu::history::ObservedCards {
                observed_at: None,
                cards: [Some(1), None, Some(3)],
                level: Some(11),
                partial: true,
                kind: "changed".into(),
            }],
        };
        assert!(save_observation(&path, &b, observation.clone()).is_err());
        let view = save_observation(&path, &a, observation.clone()).unwrap();
        assert_eq!(view.observations.len(), 1);
        let bytes = std::fs::read(&path).unwrap();
        save_observation(&path, &a, observation).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        merge(&path, &a, vec![row(2)], false).unwrap();
        assert_eq!(offline(&path, 0).unwrap().observations.len(), 1);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
    fn owner(id: &str, platform: &str) -> Owner {
        Owner::from_values(&serde_json::json!({"puuid":id,"platformId":platform}), &[]).unwrap()
    }
    fn row(id: u64) -> Match {
        Match {
            game_id: id,
            created: id,
            duration: 600,
            champion: 1,
            kills: 1,
            deaths: 2,
            assists: 3,
            result: "win".into(),
            items: vec![],
            augments: vec![],
        }
    }
    #[test]
    fn failed_query_keeps_bytes_and_account_switch_hides_old_cache() {
        struct Fake {
            before: Owner,
            after: Owner,
            calls: std::cell::Cell<u32>,
        }
        impl Source for Fake {
            fn owner(&self) -> Result<Owner> {
                let n = self.calls.get();
                self.calls.set(n + 1);
                Ok(if n == 0 {
                    self.before.clone()
                } else {
                    self.after.clone()
                })
            }
            fn rows(&self, _: &Owner) -> Result<Vec<Match>> {
                Err("history-request-failed")
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "fuze-history-failure-fixture-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.json");
        let a = owner("fixture-account-A", "HN1");
        let b = owner("fixture-account-B", "HN1");
        merge(&path, &a, vec![row(1)], false).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let v = refresh_from(
            &Fake {
                before: a.clone(),
                after: a.clone(),
                calls: 0.into(),
            },
            &path,
        );
        assert!(v.cached);
        assert_eq!(v.rows.len(), 1);
        assert_eq!(v.error, Some("history-request-failed"));
        let v = refresh_from(
            &Fake {
                before: a,
                after: b,
                calls: 0.into(),
            },
            &path,
        );
        assert!(v.rows.is_empty());
        assert_eq!(v.error, Some("history-account-changed"));
        assert_eq!(bytes, std::fs::read(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
    #[test]
    fn cache_isolates_accounts_regions_preserves_newer_and_invalid_file() {
        let dir = std::env::temp_dir().join(format!("fuze-history-fixture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.json");
        let a = owner("fixture-account-A", "HN1");
        let b = owner("fixture-account-B", "HN1");
        let c = owner("fixture-account-A", "HN2");
        merge(&path, &a, vec![row(1)], false).unwrap();
        assert!(cached(&path, &b).unwrap().is_empty());
        assert!(cached(&path, &c).unwrap().is_empty());
        let mut older = row(1);
        older.kills = 99;
        merge(&path, &a, vec![older, row(2)], true).unwrap();
        assert_eq!(cached(&path, &a).unwrap()[1].kills, 1);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("fixture-account"));
        std::fs::write(&path, b"invalid fixture").unwrap();
        assert!(merge(&path, &a, vec![row(3)], false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid fixture");
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
