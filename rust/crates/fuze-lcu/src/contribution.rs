//! Normalize only a verified owner's completed SGP games. Never log raw responses.
use crate::history::{Owner, Result};
use serde_json::{json, Value};

pub struct Page {
    pub rows: Vec<crate::history::Match>,
    pub games: Vec<Value>,
}
fn number(v: &Value, max: u64) -> Option<u64> {
    v.as_u64().filter(|v| *v <= max)
}
fn text(v: &Value, max: usize) -> Option<&str> {
    v.as_str()
        .filter(|s| s.len() <= max && !s.chars().any(char::is_control))
}
fn optional_text(v: Option<&Value>, max: usize) -> Option<&str> {
    match v {
        None | Some(Value::Null) => Some(""),
        Some(v) => text(v, max),
    }
}
fn participant(p: &Value) -> Option<Value> {
    let puuid = match p.get("puuid") {
        None | Some(Value::Null) => Value::Null,
        Some(v) => {
            let s = text(v, 128)?;
            if s.len() < 8 || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return None;
            }
            v.clone()
        }
    };
    let list = |prefix: &str, start: u32, end: u32| -> Option<Vec<u64>> {
        (start..=end)
            .map(|i| match p.get(format!("{prefix}{i}")) {
                None => Some(0),
                Some(v) => number(v, u32::MAX as u64),
            })
            .collect::<Option<Vec<_>>>()
            .map(|v| v.into_iter().filter(|v| *v > 0).collect())
    };
    Some(json!({
        "participantId": number(&p["participantId"], 20).filter(|id| *id > 0)?,
        "puuid": puuid,
        "gameName": optional_text(p.get("riotIdGameName").or_else(|| p.get("gameName")), 128)?,
        "tagLine": optional_text(p.get("riotIdTagline").or_else(|| p.get("tagLine")), 32)?,
        "championId": number(&p["championId"], u32::MAX as u64).filter(|n| *n > 0)?,
        "teamId": number(&p["teamId"], u32::MAX as u64)?,
        "playerSubteamId": number(p.get("playerSubteamId").unwrap_or(&json!(0)), u32::MAX as u64)?,
        "subteamPlacement": number(p.get("subteamPlacement").unwrap_or(&json!(0)), u32::MAX as u64)?,
        "win": p["win"].as_bool()?,
        "gameEndedInEarlySurrender": p.get("gameEndedInEarlySurrender").map(Value::as_bool).unwrap_or(Some(false))?,
        "kills": number(&p["kills"], 10000)?,
        "deaths": number(&p["deaths"], 10000)?,
        "assists": number(&p["assists"], 10000)?,
        "items": list("item", 0, 6)?,
        "augments": list("playerAugment", 1, 6)?
    }))
}
pub fn normalize(owner: &Owner, game: &Value, patch: &str) -> Option<Value> {
    let version = text(&game["gameVersion"], 64)?;
    if game["gameMode"] != "KIWI"
        || game["queueId"] != 2400
        || game["mapId"] != 12
        || game["endOfGameResult"] != "GameComplete"
        || !(version == patch || version.starts_with(&format!("{patch}.")))
        || game["platformId"] != owner.platform()
    {
        return None;
    }
    let players = game["participants"]
        .as_array()
        .filter(|p| !p.is_empty() && p.len() <= 20)?;
    if players.iter().filter(|p| owner.owns_participant(p)).count() != 1 {
        return None;
    }
    let mut ids = std::collections::HashSet::new();
    let participants: Option<Vec<_>> = players
        .iter()
        .map(|p| {
            let p = participant(p)?;
            ids.insert(p["participantId"].as_u64()?).then_some(p)
        })
        .collect();
    let mut participants = participants?;
    participants.sort_by_key(|p| p["participantId"].as_u64());
    let mutators = game.get("gameModeMutators").cloned().unwrap_or(json!([]));
    if !mutators
        .as_array()
        .is_some_and(|a| a.len() <= 16 && a.iter().all(|v| text(v, 100).is_some()))
    {
        return None;
    }
    Some(
        json!({"platformId":owner.platform(),"gameId":number(&game["gameId"],9_007_199_254_740_991).filter(|n|*n>0)?,
        "gameCreation":number(&game["gameCreation"],9_007_199_254_740_991).filter(|n|*n>0)?,
        "gameDuration":number(&game["gameDuration"],14400)?,"gameMode":"KIWI","gameModeMutators":mutators,
        "gameType":text(&game["gameType"],64)?,"gameVersion":version,"mapId":12,"queueId":2400,
        "endOfGameResult":"GameComplete","participants":participants}),
    )
}
pub fn page(owner: &Owner, payload: &Value, patch: Option<&str>) -> Result<Page> {
    let rows = crate::history::parse_sgp(owner, payload)?;
    let mut games: Vec<_> = patch
        .into_iter()
        .flat_map(|patch| {
            payload["games"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |g| normalize(owner, &g["json"], patch))
        })
        .collect();
    games.sort_by_key(|g| std::cmp::Reverse(g["gameCreation"].as_u64()));
    games.truncate(10);
    Ok(Page { rows, games })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_owned_complete_current_games_and_allowlisted_fields() {
        let owner = Owner::from_values(&json!({"puuid":"fixture-owner","platformId":"GZ100"}), &[])
            .unwrap();
        let game = json!({"platformId":"GZ100","gameId":1,"gameCreation":1000,"gameDuration":900,"gameMode":"KIWI","queueId":2400,"mapId":12,"gameVersion":"16.21.1","gameType":"MATCHED_GAME","endOfGameResult":"GameComplete","secret":"never-retain","participants":[{"participantId":1,"puuid":"fixture-owner","riotIdGameName":"Synthetic","riotIdTagline":"TEST","championId":21,"teamId":100,"win":true,"kills":1,"deaths":2,"assists":3,"item0":1001,"playerAugment1":7,"token":"never-retain"}]});
        let good = normalize(&owner, &game, "16.21").unwrap();
        assert_eq!(good["participants"][0]["gameName"], "Synthetic");
        assert!(!good.to_string().contains("never-retain"));
        for (key, value) in [
            ("platformId", json!("HN1")),
            ("gameMode", json!("ARAM")),
            ("queueId", json!(450)),
            ("endOfGameResult", json!("InProgress")),
            ("gameVersion", json!("16.20.4")),
            ("gameVersion", json!("16.210.1")),
        ] {
            let mut bad = game.clone();
            bad[key] = value;
            assert!(normalize(&owner, &bad, "16.21").is_none());
        }
        let mut other = game.clone();
        other["participants"][0]["puuid"] = json!("other-owner");
        assert!(normalize(&owner, &other, "16.21").is_none());
        assert!(page(&owner, &json!({"games":[{"json":game}]}), None)
            .unwrap()
            .games
            .is_empty());
    }
}
