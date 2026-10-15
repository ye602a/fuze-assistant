//! Bounded current-account history. Raw identity payloads never leave this boundary.
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub type Result<T> = std::result::Result<T, &'static str>;
#[derive(Clone, PartialEq, Eq)]
pub struct Owner {
    puuid: String,
    platform: String,
}
impl std::fmt::Debug for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Owner([redacted])")
    }
}
pub fn platform(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim().to_ascii_uppercase();
    ((2..=16).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !["CN", "TENCENT", "UNKNOWN"].contains(&s.as_str()))
    .then_some(s)
}
fn reported(v: &Value) -> Result<Option<String>> {
    let a = platform(&v["currentPlatformId"]);
    let b = platform(&v["platformId"]);
    if a.is_some() && b.is_some() && a != b {
        return Err("history-platform-conflict");
    }
    Ok(a.or(b))
}
impl Owner {
    pub fn from_values(summoner: &Value, routing: &[Value]) -> Result<Self> {
        let puuid = summoner["puuid"]
            .as_str()
            .filter(|s| {
                (8..=128).contains(&s.len())
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
            .ok_or("history-login-required")?;
        let mut found = reported(summoner)?;
        for v in routing {
            if let Some(next) = reported(v)? {
                if found.as_ref().is_some_and(|old| old != &next) {
                    return Err("history-platform-conflict");
                }
                found = Some(next);
            }
        }
        Ok(Self {
            puuid: puuid.into(),
            platform: found.ok_or("history-platform-unknown")?,
        })
    }
    pub fn platform(&self) -> &str {
        &self.platform
    }
    /// Call only at an account boundary, not per OCR frame. Never persist these bytes.
    pub fn cache_material(&self) -> Vec<u8> {
        format!("fuze-history-v1:{}:{}", self.platform, self.puuid).into_bytes()
    }
    pub fn matches_legacy(&self, v: &Value) -> bool {
        v["puuid"].as_str() == Some(&self.puuid)
            && platform(&v["platformId"]).as_deref() == Some(&self.platform)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Match {
    pub game_id: u64,
    pub created: u64,
    pub duration: u32,
    pub champion: u32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub result: String,
    pub items: Vec<u32>,
    pub augments: Vec<u32>,
}
impl Match {
    pub fn valid(&self) -> bool {
        self.game_id > 0
            && self.game_id <= 9_007_199_254_740_991
            && self.champion > 0
            && self.created <= 9_007_199_254_740_991
            && self.duration <= 86400
            && self.kills <= 10000
            && self.deaths <= 10000
            && self.assists <= 10000
            && ["win", "loss", "remake"].contains(&self.result.as_str())
            && self.items.len() <= 12
            && self.augments.len() <= 12
            && self.items.iter().chain(&self.augments).all(|id| *id > 0)
    }
}
fn number(v: &Value) -> Result<u64> {
    v.as_u64().ok_or("history-invalid-response")
}
fn small(v: &Value) -> Result<u32> {
    u32::try_from(number(v)?).map_err(|_| "history-invalid-response")
}
pub fn parse(owner: &Owner, v: &Value) -> Result<Vec<Match>> {
    if reported(v)?.is_some_and(|s| s != owner.platform) {
        return Err("history-account-changed");
    }
    let games = v["games"]["games"]
        .as_array()
        .filter(|g| g.len() <= 50)
        .ok_or("history-invalid-response")?;
    let mut out = Vec::new();
    for g in games {
        if reported(g)?.is_some_and(|s| s != owner.platform) {
            return Err("history-account-changed");
        }
        // Exact Mayhem queue only; never label generic ARAM results as Mayhem.
        if g["queueId"].as_u64() != Some(2400) {
            continue;
        }
        let participants = g["participants"]
            .as_array()
            .filter(|a| a.len() <= 20)
            .ok_or("history-invalid-response")?;
        let identities = g["participantIdentities"]
            .as_array()
            .filter(|a| a.len() <= 20)
            .ok_or("history-invalid-response")?;
        let owned: Vec<_> = identities
            .iter()
            .filter(|i| i["player"]["puuid"].as_str() == Some(&owner.puuid))
            .collect();
        if owned.len() != 1 {
            return Err("history-owner-mismatch");
        }
        let id = number(&owned[0]["participantId"])?;
        let mine: Vec<_> = participants
            .iter()
            .filter(|p| p["participantId"].as_u64() == Some(id))
            .collect();
        if mine.len() != 1 {
            return Err("history-invalid-response");
        }
        let p = mine[0];
        let s = &p["stats"];
        let ids = |prefix: &str, start: u32, end: u32| -> Result<Vec<u32>> {
            let mut out = Vec::new();
            for i in start..=end {
                if let Some(v) = s.get(format!("{prefix}{i}")) {
                    let id = small(v)?;
                    if id > 0 {
                        out.push(id);
                    }
                }
            }
            Ok(out)
        };
        let win = s["win"].as_bool().ok_or("history-invalid-response")?;
        let m = Match {
            game_id: number(&g["gameId"])?,
            created: number(&g["gameCreation"])?,
            duration: small(&g["gameDuration"])?,
            champion: small(&p["championId"])?,
            kills: small(&s["kills"])?,
            deaths: small(&s["deaths"])?,
            assists: small(&s["assists"])?,
            result: if s["gameEndedInEarlySurrender"] == true {
                "remake"
            } else if win {
                "win"
            } else {
                "loss"
            }
            .into(),
            items: ids("item", 0, 6)?,
            augments: ids("playerAugment", 1, 6)?,
        };
        if !m.valid() {
            return Err("history-invalid-response");
        }
        out.push(m);
    }
    Ok(out)
}
pub fn legacy(owner: &Owner, v: &Value) -> Result<Vec<Match>> {
    if v["version"] != 1 {
        return Err("history-legacy-schema");
    }
    let accounts = v["accounts"]
        .as_array()
        .filter(|a| a.len() <= 5)
        .ok_or("history-legacy-schema")?;
    let found: Vec<_> = accounts
        .iter()
        .filter(|a| owner.matches_legacy(a))
        .collect();
    if found.len() != 1 {
        return Err("history-legacy-owner");
    }
    let rows = found[0]["matches"]
        .as_array()
        .filter(|a| a.len() <= 500)
        .ok_or("history-legacy-schema")?;
    let mut out = Vec::new();
    for r in rows {
        let ids = |key: &str| -> Result<Vec<u32>> {
            r[key]
                .as_array()
                .filter(|a| a.len() <= 12)
                .ok_or("history-legacy-schema")?
                .iter()
                .map(|a| small(&a["id"]))
                .collect()
        };
        let m = Match {
            game_id: number(&r["gameId"])?,
            created: number(&r["gameCreation"])?,
            duration: small(&r["gameDuration"])?,
            champion: small(&r["championId"])?,
            kills: small(&r["kills"])?,
            deaths: small(&r["deaths"])?,
            assists: small(&r["assists"])?,
            result: r["result"].as_str().ok_or("history-legacy-schema")?.into(),
            items: ids("items")?,
            augments: ids("augments")?,
        };
        if !m.valid() {
            return Err("history-legacy-schema");
        }
        out.push(m);
    }
    Ok(out)
}
/// Candidate observations only. No chosen card, claim count or stage inference exists here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub game_id: u64,
    pub interrupted: bool,
    pub capped: bool,
    pub events: Vec<ObservedCards>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedCards {
    pub cards: [Option<u32>; 3],
    pub level: Option<u32>,
    pub partial: bool,
    pub kind: String,
}
impl Observation {
    pub fn valid(&self) -> bool {
        self.game_id > 0
            && self.game_id <= 9_007_199_254_740_991
            && self.events.len() <= 80
            && self.events.iter().all(|e| {
                e.cards.iter().flatten().all(|id| *id > 0)
                    && e.level.is_none_or(|l| (1..=30).contains(&l))
                    && ["first", "changed", "reappeared"].contains(&e.kind.as_str())
            })
    }
}
pub fn legacy_observations(owner: &Owner, v: &Value, game_id: u64) -> Result<Observation> {
    if v["version"] != 1 {
        return Err("journal-schema");
    }
    let sessions = v["sessions"]
        .as_array()
        .filter(|a| a.len() <= 100)
        .ok_or("journal-schema")?;
    let selected: Vec<_> = sessions
        .iter()
        .filter(|s| owner.matches_legacy(&s["owner"]) && s["gameId"].as_u64() == Some(game_id))
        .collect();
    if selected.len() != 1 {
        return Err("journal-owner-or-game-mismatch");
    }
    let s = selected[0];
    let events = s["events"]
        .as_array()
        .filter(|a| a.len() <= 80)
        .ok_or("journal-schema")?
        .iter()
        .map(|e| {
            let slots = e["cards"]
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or("journal-schema")?;
            let mut cards = [None; 3];
            for (i, v) in slots.iter().enumerate() {
                if !v.is_null() {
                    cards[i] = Some(small(&v["id"])?);
                }
            }
            let level = if e["level"].is_null() {
                None
            } else {
                Some(small(&e["level"])?)
            };
            Ok(ObservedCards {
                cards,
                level,
                partial: e["partial"] == true || cards.iter().any(Option::is_none),
                kind: e["kind"].as_str().ok_or("journal-schema")?.into(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let out = Observation {
        game_id,
        events,
        interrupted: s["interrupted"] == true || s["endedAt"].is_null(),
        capped: s["capped"] == true,
    };
    if !out.valid() {
        return Err("journal-schema");
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn journal_selects_owner_and_game_without_claim_inference() {
        let o = Owner::from_values(
            &json!({"puuid":"fixture-account-A","platformId":"HN1"}),
            &[],
        )
        .unwrap();
        let mut v = json!({"version":1,"sessions":[
            {"gameId":1,"owner":{"puuid":"fixture-account-B","platformId":"HN1"},"events":[{}]},
            {"gameId":2,"owner":{"puuid":"fixture-account-A","platformId":"HN1"},"events":[{}]},
            {"gameId":1,"owner":{"puuid":"fixture-account-A","platformId":"HN1","playerName":"discarded"},"endedAt":null,"capped":true,
            "events":[{"kind":"changed","level":11,"cards":[{"id":1},null,{"id":3}],"observedAt":123,"chosen":3},
                      {"kind":"reappeared","level":11,"cards":[{"id":1},null,{"id":3}]}]}
        ]});
        let result = legacy_observations(&o, &v, 1).unwrap();
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].cards, [Some(1), None, Some(3)]);
        assert!(result.interrupted && result.capped && result.events[0].partial);
        let json = serde_json::to_string(&result).unwrap();
        for key in ["puuid", "playerName", "chosen", "observedAt", "claim"] {
            assert!(!json.contains(key));
        }
        assert!(legacy_observations(&o, &v, 3).is_err());
        v["sessions"][2]["events"][0]["cards"] = json!([{"id":1},{"id":2}]);
        assert!(legacy_observations(&o, &v, 1).is_err());
    }
    #[test]
    fn legacy_import_only_selects_exact_account_and_region() {
        let owner = Owner::from_values(
            &json!({"puuid":"fixture-account-A","platformId":"HN1"}),
            &[],
        )
        .unwrap();
        let row = json!({"gameId":1,"gameCreation":100,"gameDuration":600,"championId":21,"kills":1,"deaths":2,"assists":3,"result":"win","items":[{"id":1001,"name":"ignored"}],"augments":[]});
        let mut value = json!({"version":1,"accounts":[
            {"puuid":"fixture-account-B","platformId":"HN1","matches":[{}]},
            {"puuid":"fixture-account-A","platformId":"HN2","matches":[{}]},
            {"puuid":"fixture-account-A","platformId":"HN1","playerName":"discarded","matches":[row]}
        ]});
        assert_eq!(legacy(&owner, &value).unwrap().len(), 1);
        let duplicate = value["accounts"][2].clone();
        value["accounts"].as_array_mut().unwrap().push(duplicate);
        assert_eq!(legacy(&owner, &value).unwrap_err(), "history-legacy-owner");
    }
    #[test]
    fn route_must_be_explicit_and_consistent() {
        let s = json!({"puuid":"fixture-account-A","platformId":"CN"});
        assert!(Owner::from_values(&s, &[]).is_err());
        assert!(Owner::from_values(
            &s,
            &[json!({"platformId":"HN1"}), json!({"platformId":"HN2"})]
        )
        .is_err());
        let owner = Owner::from_values(&s, &[json!({"platformId":"HN1"})]).unwrap();
        assert!(!format!("{owner:?}").contains("fixture-account"));
        assert!(parse(&owner, &json!({"platformId":"HN2","games":{"games":[]}})).is_err());
    }
    #[test]
    fn reduces_only_owner_and_rejects_cross_account() {
        let o = Owner::from_values(
            &json!({"puuid":"fixture-account-A","platformId":"HN1"}),
            &[],
        )
        .unwrap();
        let mut v = json!({"games":{"games":[{"queueId":2400,"platformId":"HN1","gameId":1,"gameCreation":100,"gameDuration":500,"participantIdentities":[{"participantId":1,"player":{"puuid":"fixture-account-A","summonerName":"private-name"}}],"participants":[{"participantId":1,"championId":1,"stats":{"win":true,"kills":1,"deaths":2,"assists":3,"item0":10,"playerAugment1":20}}]}]}});
        let rows = parse(&o, &v).unwrap();
        assert_eq!(rows[0].augments, vec![20]);
        let encoded = serde_json::to_string(&rows).unwrap();
        assert!(!encoded.contains("private-name"));
        assert!(!encoded.contains("puuid"));
        v["games"]["games"][0]["participantIdentities"][0]["player"]["puuid"] =
            json!("fixture-account-B");
        assert!(parse(&o, &v).is_err());
    }
}
