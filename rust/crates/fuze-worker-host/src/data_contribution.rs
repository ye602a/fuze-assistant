//! Own post-game samples only. No production HTTP transport is wired in this stage.
use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
pub const SESSION_PATH: &str = "/api/client/v1/match-history/upload-session";
pub const BATCH_PATH: &str = "/api/client/v1/match-history/batches";
pub const TARGET_PATCH: &str = "16.21"; // Simulation snapshot; live activation must read collectionPolicy.
#[derive(Serialize, Deserialize)]
struct Setting {
    enabled: bool,
}
pub fn enabled(directory: &Path) -> Result<bool> {
    let path = directory.join("data-contribution-setting.json");
    if !path.exists() {
        return Ok(true);
    }
    let s: Setting = serde_json::from_slice(&crate::update_channel::bounded(&path, 1024)?)
        .map_err(|_| "contribution-setting")?;
    Ok(s.enabled)
}
pub fn set_enabled(directory: &Path, enabled: bool) -> Result<()> {
    crate::preferences::save_json(
        &directory.join("data-contribution-setting.json"),
        &Setting { enabled },
    )
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub pending: usize,
    pub uploaded: usize,
    pub rejected: usize,
    pub notice: String,
}
impl Summary {
    pub fn label(&self) -> String {
        format!(
            "待传 {} 局 · 已确认 {} 局 · 未接受 {} 局\n{}",
            self.pending,
            self.uploaded,
            self.rejected,
            if self.notice.is_empty() {
                "真实上传未开放；不会向数据站发送对局。"
            } else {
                &self.notice
            }
        )
    }
}
#[derive(Serialize, Deserialize)]
struct Entry {
    source: String,
    hash: String,
    key: String,
    observed_at: String,
    game: Option<Value>,
    rejected: bool,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Queue {
    entries: Vec<Entry>,
}
impl Queue {
    pub fn read(directory: &Path) -> Result<Self> {
        let path = directory.join("data-contribution-outbox.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&crate::update_channel::bounded(&path, 32 * 1024 * 1024)?)
            .map_err(|_| "contribution-outbox")
    }
    fn save(&self, directory: &Path) -> Result<()> {
        crate::preferences::save_json(&directory.join("data-contribution-outbox.json"), self)
    }
    pub fn summary(&self) -> Summary {
        Summary {
            pending: self
                .entries
                .iter()
                .filter(|e| e.game.is_some() && !e.rejected)
                .count(),
            uploaded: self
                .entries
                .iter()
                .filter(|e| e.game.is_none() && !e.rejected)
                .count(),
            rejected: self.entries.iter().filter(|e| e.rejected).count(),
            notice: String::new(),
        }
    }
    fn enqueue(&mut self, games: Vec<Value>, observed_at: &str) -> Result<()> {
        for game in games {
            let source = format!(
                "match-history:v1:{}:{}",
                game["platformId"].as_str().ok_or("contribution-game")?,
                game["gameId"].as_u64().ok_or("contribution-game")?
            );
            let bytes = serde_json::to_vec(&game).map_err(|_| "contribution-game")?;
            let hash = crate::update_io::hex(&crate::update_io::digest(&bytes, false)?);
            if self
                .entries
                .iter()
                .any(|e| e.source == source && e.hash == hash)
            {
                continue;
            }
            if self.summary().pending >= 200 || self.entries.len() >= 50000 {
                return Err("contribution-queue-full");
            }
            // Stable across retries/restarts; changed payloads get a different idempotency key.
            let key = crate::update_io::hex(&crate::update_io::digest(
                format!("{source}:{hash}").as_bytes(),
                false,
            )?);
            let entry = Entry {
                source: source.clone(),
                hash,
                key,
                observed_at: observed_at.into(),
                game: Some(game),
                rejected: false,
            };
            if let Some(old) = self.entries.iter_mut().find(|e| e.source == source) {
                *old = entry;
            } else {
                self.entries.push(entry);
            }
        }
        Ok(())
    }
    /// Pure protocol boundary exercised by mocks. The application never calls this with HTTP.
    pub fn send_with(
        &mut self,
        client_version: &str,
        installation: &str,
        now: &str,
        transport: &mut impl FnMut(&str, Option<&str>, &Value) -> Result<Value>,
    ) -> Result<Summary> {
        let Some(first) = self
            .entries
            .iter()
            .find(|e| e.game.is_some() && !e.rejected)
        else {
            return Ok(self.summary());
        };
        let platform = first.game.as_ref().unwrap()["platformId"]
            .as_str()
            .ok_or("contribution-game")?
            .to_owned();
        let session = transport(
            SESSION_PATH,
            None,
            &json!({"schemaVersion":2,"clientVersion":client_version,"platformId":platform,"installationId":installation}),
        )?;
        let token = session["token"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 16384)
            .ok_or("contribution-session")?;
        let limit = session["maxBatchSize"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("contribution-session")?
            .min(20) as usize;
        let samples: Vec<_> = self
            .entries
            .iter()
            .filter(|e| !e.rejected && e.game.as_ref().is_some_and(|g| g["platformId"] == platform))
            .take(limit)
            .map(|e| {
                json!({
                    "sourceKey": e.source, "idempotencyKey": e.key, "payloadHash": e.hash,
                    "observedAt": e.observed_at, "game": e.game
                })
            })
            .collect();
        let batch = json!({"schemaVersion":2,"clientVersion":client_version,"sentAt":now,"pendingUploadCount":self.summary().pending,"samples":samples});
        let max = session["maxBodyBytes"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("contribution-session")?;
        if serde_json::to_vec(&batch)
            .map_err(|_| "contribution-game")?
            .len() as u64
            > max
        {
            return Err("contribution-body-limit");
        }
        let response = transport(BATCH_PATH, Some(token), &batch)?;
        let acks = response["acknowledgements"]
            .as_array()
            .ok_or("contribution-ack")?;
        for sample in samples {
            let matching: Vec<_> = acks
                .iter()
                .filter(|a| {
                    a["sourceKey"] == sample["sourceKey"]
                        && a["idempotencyKey"] == sample["idempotencyKey"]
                })
                .collect();
            if matching.len() != 1 {
                continue;
            }
            let ack = matching[0];
            let e = self
                .entries
                .iter_mut()
                .find(|e| e.key == sample["idempotencyKey"].as_str().unwrap())
                .unwrap();
            match ack["status"].as_str() {
                Some("inserted" | "duplicate" | "updated") => e.game = None,
                Some("rejected") if ack["retryable"] == false && ack["code"] != "game_archived" => {
                    e.rejected = true;
                    e.game = None;
                }
                _ => {}
            }
        }
        Ok(self.summary())
    }
}
pub fn stage(directory: &Path, games: Vec<Value>) -> Result<Summary> {
    if !enabled(directory)? {
        return Ok(Summary {
            notice: "数据贡献已关闭，不收集新样本。".into(),
            ..Default::default()
        });
    }
    let mut queue = Queue::read(directory)?;
    if games.is_empty() {
        return Ok(Summary {
            notice: "没有符合当前采集补丁与完整性要求的新样本；尚未实际上传。".into(),
            ..queue.summary()
        });
    }
    let mut utc = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetSystemTime(&mut utc) };
    let observed_at = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        utc.wYear, utc.wMonth, utc.wDay, utc.wHour, utc.wMinute, utc.wSecond
    );
    queue.enqueue(games, &observed_at)?;
    queue.save(directory)?;
    Ok(queue.summary())
}
pub fn status(directory: &Path, notice: Option<&str>) -> Summary {
    match Queue::read(directory) {
        Ok(queue) => Summary {
            notice: notice.unwrap_or_default().into(),
            ..queue.summary()
        },
        Err(_) => Summary {
            notice: "贡献队列无法读取，原文件已保留；计数暂不可用。".into(),
            ..Default::default()
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const OBSERVED: &str = "2026-10-23T00:00:00Z";
    fn game(id: u64) -> Value {
        json!({"platformId":"GZ100","gameId":id,"participants":[{"championId":21}]})
    }
    #[test]
    fn synthetic_sgp_page_reaches_mock_ack_without_network() {
        let owner = fuze_lcu::history::Owner::from_values(
            &json!({"puuid":"fixture-owner","platformId":"GZ100"}),
            &[],
        )
        .unwrap();
        let player = |id, puuid| {
            json!({"participantId":id,"puuid":puuid,
            "riotIdGameName":"Synthetic","riotIdTagline":"TEST","championId":21,
            "teamId":100,"win":true,"kills":1,"deaths":2,"assists":3,
            "item0":1001,"playerAugment1":7,"token":"must-not-escape"})
        };
        let raw = json!({"platformId":"GZ100","gameId":7,"gameCreation":1000,
            "gameDuration":900,"gameMode":"KIWI","queueId":2400,"mapId":12,
            "gameVersion":"16.21.1","gameType":"MATCHED_GAME","endOfGameResult":"GameComplete",
            "participants":[player(1,"fixture-owner"),player(2,"fixture-other")]});
        let page = fuze_lcu::contribution::page(
            &owner,
            &json!({"games":[{"json":raw}]}),
            Some(TARGET_PATCH),
        )
        .unwrap();
        assert_eq!((page.rows.len(), page.games.len()), (1, 1));
        let mut queue = Queue::default();
        queue.enqueue(page.games, OBSERVED).unwrap();
        let summary = queue.send_with("0.7.0", "11111111-1111-4111-8111-111111111111", OBSERVED,
            &mut |path, _, body| {
                if path == SESSION_PATH {
                    return Ok(json!({"token":"mock-short-lived-token","expiresAt":"2099-01-01T00:00:00Z","maxBatchSize":1,"maxBodyBytes":65536}));
                }
                let s = &body["samples"][0];
                assert_eq!(s["sourceKey"], "match-history:v1:GZ100:7");
                assert_eq!(s["game"]["participants"].as_array().unwrap().len(), 2);
                assert_eq!(s["game"]["participants"][0]["augments"], json!([7]));
                assert!(!body.to_string().contains("must-not-escape"));
                Ok(json!({"acknowledgements":[{"sourceKey":s["sourceKey"],
                    "idempotencyKey":s["idempotencyKey"],"status":"inserted"}]}))
            }).unwrap();
        assert_eq!((summary.pending, summary.uploaded), (0, 1));
        assert!(queue.entries[0].game.is_none());
    }
    #[test]
    fn mock_protocol_deduplicates_retries_and_matches_each_receipt() {
        let mut queue = Queue::default();
        queue
            .enqueue(vec![game(1), game(1), game(2)], OBSERVED)
            .unwrap();
        assert_eq!(queue.summary().pending, 2);
        let before = queue.entries[0].key.clone();
        assert!(queue
            .send_with(
                "0.7.0",
                "11111111-1111-4111-8111-111111111111",
                "2026-10-23T00:00:00Z",
                &mut |_, _, _| Err("mock-network")
            )
            .is_err());
        assert_eq!(queue.entries[0].key, before);
        assert_eq!(queue.summary().uploaded, 0);
        let mut calls = 0;
        let mut transport = |path: &str, token: Option<&str>, body: &Value| {
            calls += 1;
            if path == SESSION_PATH {
                assert!(token.is_none());
                assert_eq!(body["schemaVersion"], 2);
                assert_eq!(body["platformId"], "GZ100");
                return Ok(
                    json!({"token":"mock-short-lived-token","expiresAt":"2099-01-01T00:00:00Z","maxBatchSize":20,"maxBodyBytes":1048576}),
                );
            }
            assert_eq!(path, BATCH_PATH);
            assert_eq!(token, Some("mock-short-lived-token"));
            let samples = body["samples"].as_array().unwrap();
            assert_eq!(samples.len(), 2);
            assert_eq!(samples[0]["observedAt"], OBSERVED);
            Ok(
                json!({"serverTime":"2026-10-23T00:00:00Z","acknowledgements":[
                {"sourceKey":samples[0]["sourceKey"],"idempotencyKey":"wrong-key","status":"inserted"},
                {"sourceKey":samples[1]["sourceKey"],"idempotencyKey":samples[1]["idempotencyKey"],"status":"duplicate"}]}),
            )
        };
        let summary = queue
            .send_with(
                "0.7.0",
                "11111111-1111-4111-8111-111111111111",
                "2026-10-23T01:00:00Z",
                &mut transport,
            )
            .unwrap();
        assert_eq!((summary.pending, summary.uploaded, calls), (1, 1, 2));
        queue.enqueue(vec![game(1), game(2)], OBSERVED).unwrap();
        assert_eq!(queue.summary().pending, 1);
        // Acknowledged tombstones survive serialization, without retaining the game body.
        let mut restored: Queue =
            serde_json::from_slice(&serde_json::to_vec(&queue).unwrap()).unwrap();
        assert!(restored.entries[1].game.is_none());
        let mut changed = game(2);
        changed["participants"][0]["championId"] = json!(22);
        restored.enqueue(vec![changed], OBSERVED).unwrap();
        assert_eq!(restored.summary().pending, 2);
    }
    #[test]
    fn mock_receipts_stop_permanent_rejections_and_retain_retryable_samples() {
        let mut queue = Queue::default();
        queue
            .enqueue(vec![game(1), game(2), game(3)], OBSERVED)
            .unwrap();
        let summary = queue.send_with("0.7.0", "11111111-1111-4111-8111-111111111111", OBSERVED,
            &mut |path, _, body| {
                if path == SESSION_PATH {
                    return Ok(json!({"token":"mock-short-lived-token","expiresAt":"2099-01-01T00:00:00Z","maxBatchSize":3,"maxBodyBytes":1048576}));
                }
                let acks: Vec<_> = body["samples"].as_array().unwrap().iter().enumerate()
                    .map(|(i, s)| json!({"sourceKey":s["sourceKey"],"idempotencyKey":s["idempotencyKey"],
                        "status":"rejected","retryable": i == 1,"code":if i == 2 {"game_archived"} else {"invalid_sample"}}))
                    .collect();
                Ok(json!({"acknowledgements":acks}))
            }).unwrap();
        assert_eq!(
            (summary.pending, summary.rejected, summary.uploaded),
            (2, 1, 0)
        );
        assert!(queue.entries[0].game.is_none());
        queue.enqueue(vec![game(1)], OBSERVED).unwrap();
        assert_eq!(queue.summary().pending, 2);
    }
    #[test]
    fn opt_out_survives_restart_and_production_stages_without_network() {
        let root = std::env::temp_dir().join(format!(
            "fuze-contribution-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        assert!(enabled(&root).unwrap());
        assert_eq!(stage(&root, vec![game(1)]).unwrap().pending, 1);
        assert_eq!(stage(&root, vec![game(1)]).unwrap().pending, 1);
        set_enabled(&root, false).unwrap();
        assert!(!enabled(&root).unwrap());
        stage(&root, vec![game(2)]).unwrap();
        assert_eq!(Queue::read(&root).unwrap().summary().pending, 1);
        assert_eq!(Queue::read(&root).unwrap().summary().uploaded, 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
