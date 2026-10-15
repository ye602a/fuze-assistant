//! Keep only the local champion and an opaque game discriminator; no account payload escapes.
use crate::{Error, Result};
use fuze_runtime::session::Phase;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq)]
pub struct Context {
    pub phase: Phase,
    pub champion: Option<u32>,
    pub supported: bool,
    game: u64,
}
impl Context {
    pub fn idle(phase: Phase) -> Self {
        Self {
            phase,
            champion: None,
            supported: false,
            game: 0,
        }
    }
    pub fn same_game(&self, other: &Self) -> bool {
        self.game == other.game
    }
}
fn json(body: &[u8]) -> Result<Value> {
    if body.len() > 256 * 1024 {
        return Err(Error::ResponseLimit);
    }
    serde_json::from_slice(body).map_err(|_| Error::InvalidPhase)
}
fn champion(v: &Value) -> Option<u32> {
    v.as_u64()
        .filter(|v| *v > 0 && *v <= u32::MAX as u64)
        .map(|v| v as u32)
}
/// Bodies are temporary caller-owned memory, never diagnostics or persisted state.
pub fn parse(phase: Phase, gameflow: &[u8], player: &[u8]) -> Result<Context> {
    let flow = json(gameflow)?;
    if Phase::from_lcu(flow.get("phase").and_then(Value::as_str)) != phase {
        return Err(Error::InvalidPhase); // A phase transition raced the read.
    }
    let game = &flow["gameData"];
    let supported = game["queue"]["id"].as_u64() == Some(2400);
    if !supported {
        return Ok(Context::idle(phase));
    }
    let local = json(player)?;
    let selected = if phase == Phase::ChampSelect {
        let cell = local["localPlayerCellId"].as_i64().filter(|v| *v >= 0);
        let rows = local["myTeam"].as_array().ok_or(Error::InvalidPhase)?;
        if rows.len() > 10 {
            return Err(Error::ResponseLimit);
        }
        let matches: Vec<_> = rows
            .iter()
            .filter(|v| cell.is_some() && v["cellId"].as_i64() == cell)
            .collect();
        if matches.len() != 1 {
            return Err(Error::InvalidPhase);
        }
        champion(&matches[0]["championId"])
    } else {
        let id = local["summonerId"]
            .as_u64()
            .filter(|v| *v > 0)
            .ok_or(Error::InvalidPhase)?;
        let mut selected = None;
        let mut count = 0;
        for team in ["teamOne", "teamTwo"] {
            let rows = game[team].as_array().ok_or(Error::InvalidPhase)?;
            if rows.len() > 10 {
                return Err(Error::ResponseLimit);
            }
            for row in rows {
                if row["summonerId"].as_u64() == Some(id) {
                    count += 1;
                    selected = champion(&row["championId"]);
                }
            }
        }
        if count != 1 {
            return Err(Error::InvalidPhase);
        }
        selected
    };
    let key = game["gameId"].as_u64().unwrap_or(0);
    if phase == Phase::InProgress && key == 0 {
        return Err(Error::InvalidPhase);
    }
    Ok(Context {
        phase,
        champion: selected,
        supported,
        game: key,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn select_changes_and_mid_game_identity_are_not_guessed() {
        let flow = br#"{"phase":"ChampSelect","gameData":{"queue":{"id":2400}}}"#;
        let first = parse(Phase::ChampSelect,flow,br#"{"localPlayerCellId":2,"myTeam":[{"cellId":1,"championId":9},{"cellId":2,"championId":8}]}"#).unwrap();
        assert_eq!(first.champion, Some(8));
        assert!(parse(Phase::InProgress, flow, b"{}").is_err());
        let flow = br#"{"phase":"InProgress","gameData":{"queue":{"id":2400},"gameId":12,"teamOne":[{"summonerId":7,"championId":8}],"teamTwo":[]}}"#;
        assert_eq!(
            parse(Phase::InProgress, flow, br#"{"summonerId":7}"#)
                .unwrap()
                .champion,
            Some(8)
        );
        assert!(parse(Phase::InProgress, flow, br#"{"summonerId":6}"#).is_err());
        let other = String::from_utf8(flow.to_vec())
            .unwrap()
            .replace("2400", "450");
        assert!(
            !parse(Phase::InProgress, other.as_bytes(), b"{}")
                .unwrap()
                .supported
        );
    }
}
