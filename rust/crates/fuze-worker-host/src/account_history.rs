//! Manual, bounded history IO. No refresh timers and no OCR-thread work.
use crate::Result;
use fuze_lcu::history::{Match, Owner};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub account: String,
    pub platform: String,
    pub rows: std::sync::Arc<Vec<Match>>,
    pub error: Option<&'static str>,
    pub cached: bool,
    pub pending: usize,
}
pub struct Import {
    owner: Owner,
    rows: Vec<Match>,
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
    Ok(crate::preferences::path()?.with_file_name("account-history.json"))
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
        cached: true,
        ..Default::default()
    })
}
fn read(path: &Path) -> Result<Saved> {
    if !path.exists() {
        return Ok(Saved::default());
    }
    let data: Saved =
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
        })
    {
        return Err("history-cache-invalid");
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
    let old = saved
        .accounts
        .iter()
        .position(|a| a.key == key)
        .map(|i| saved.accounts.remove(i).rows)
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
    match fuze_lcu::discovery::HistorySession::open(manual) {
        Ok(session) => refresh_from(&session, path),
        Err(error) => View {
            error: Some(error),
            ..View::default()
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
    if session.owner()? != owner {
        return Err("history-account-changed");
    }
    Ok(Import { owner, rows })
}
impl Import {
    pub fn view(&self) -> View {
        View {
            account: key(&self.owner)
                .map(|k| k[..8].to_owned())
                .unwrap_or_default(),
            platform: self.owner.platform().into(),
            pending: self.rows.len(),
            ..View::default()
        }
    }
    pub fn confirm(self, manual: Option<&Path>, path: &Path) -> Result<View> {
        let session = fuze_lcu::discovery::HistorySession::open(manual)?;
        if session.owner()? != self.owner {
            return Err("history-account-changed");
        }
        let rows = merge(path, &self.owner, self.rows, true)?;
        Ok(View {
            account: key(&self.owner)?[..8].into(),
            platform: self.owner.platform().into(),
            rows: rows.into(),
            cached: true,
            ..View::default()
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
