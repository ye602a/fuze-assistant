//! Keep only the local champion and an opaque game discriminator; no account payload escapes.
use crate::{Error, Result};
use fuze_runtime::session::Phase;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq)]
pub struct Context {
    pub phase: Phase,
    pub champion: Option<u32>,
    pub supported: bool,
    pub selection: Option<Selection>,
    game: u64,
}
impl Context {
    pub fn observed_game_id(&self) -> Option<u64> {
        (self.supported
            && self.phase == Phase::InProgress
            && self.game > 0
            && self.game <= 9_007_199_254_740_991)
            .then_some(self.game)
    }
    pub fn idle(phase: Phase) -> Self {
        Self {
            phase,
            champion: None,
            supported: false,
            selection: None,
            game: 0,
        }
    }
    pub fn same_game(&self, other: &Self) -> bool {
        self.game == other.game
    }
    /// Loading has not captured or accepted any choices. If its game key was unavailable,
    /// retaining the warmed model on InProgress does not retain an observed reward.
    pub fn finishes_loading(&self, next: &Self) -> bool {
        self.phase == Phase::Loading
            && next.phase == Phase::InProgress
            && next.game != 0
            && (self.game == 0 || self.same_game(next))
    }
}
/// Public champion choices only. Never retain player names, account IDs or raw session bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub team: [u32; 5],
    pub local_slot: Option<usize>,
    pub bench: [u32; 20],
    pub count: usize,
    /// None: ordinary selection; Some(empty): subset requested but unavailable.
    pub choices: Option<[u32; 3]>,
}
/// Only the dedicated subset endpoint proves the player's two/three cards.
/// Invalid/empty responses clear previous cards without discarding team/bench data.
pub(crate) fn subset(body: &[u8]) -> Option<[u32; 3]> {
    if body.len() > 128 {
        return None;
    }
    let value: Value = serde_json::from_slice(body).ok()?;
    let rows = value.as_array()?;
    if !(2..=3).contains(&rows.len()) {
        return None;
    }
    let mut result = [0; 3];
    for (i, row) in rows.iter().enumerate() {
        let id = champion(row)?;
        if result.contains(&id) {
            return None;
        }
        result[i] = id;
    }
    Some(result)
}
fn selection(v: &Value) -> Result<Selection> {
    let rows = v["myTeam"].as_array().ok_or(Error::InvalidPhase)?;
    if rows.len() > 5 {
        return Err(Error::ResponseLimit);
    }
    let mut rows: Vec<_> = rows.iter().collect();
    rows.sort_by_key(|r| r["cellId"].as_i64().unwrap_or(-1));
    let mut result = Selection {
        team: [0; 5],
        local_slot: None,
        bench: [0; 20],
        count: 0,
        choices: (v["allowSubsetChampionPicks"] == true).then_some([0; 3]),
    };
    let mut cells = std::collections::HashSet::new();
    for (slot, row) in rows.iter().enumerate() {
        let cell = row["cellId"].as_u64().ok_or(Error::InvalidPhase)?;
        if !cells.insert(cell) {
            return Err(Error::InvalidPhase);
        }
        result.team[slot] = champion(&row["championId"]).unwrap_or(0);
        if Some(cell) == v["localPlayerCellId"].as_u64() {
            result.local_slot = Some(slot);
        }
    }
    // The mode flag stays true after the local player finishes choosing.
    // A resolved local champion ends this choice, even if the subset endpoint retains cards.
    if result.local_slot.is_none_or(|slot| result.team[slot] != 0) {
        result.choices = None;
    }
    if v["benchEnabled"] == false {
        return Ok(result);
    }
    if let Some(bench) = v["benchChampions"].as_array() {
        if bench.len() > 20 {
            return Err(Error::ResponseLimit);
        }
        for row in bench {
            if let Some(id) = champion(&row["championId"]) {
                if !result.team.contains(&id) && !result.bench[..result.count].contains(&id) {
                    result.bench[result.count] = id;
                    result.count += 1;
                }
            }
        }
    }
    Ok(result)
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
        selection: if phase == Phase::ChampSelect {
            Some(selection(&local)?)
        } else {
            None
        },
        game: key,
    })
}
#[cfg(test)]
mod tests {
    #[test]
    fn subset_two_three_unavailable_and_new_session_do_not_reuse_cards() {
        use super::*;
        assert_eq!(subset(b"[875,103]"), Some([875, 103, 0]));
        assert_eq!(subset(b"[875,103,893]"), Some([875, 103, 893]));
        for body in [
            "[]",
            "[0,103]",
            "[103,103]",
            "[1,2,3,4]",
            "[1]",
            "[1,-2]",
            "[1,2.5]",
            "{}",
            "[1,4294967296]",
        ] {
            assert_eq!(subset(body.as_bytes()), None, "{body}");
        }
        assert_eq!(subset(&[b' '; 129]), None);
        let mut session = serde_json::json!({"myTeam":[{"cellId":2,"championId":0},{"cellId":0,"championId":0}],"localPlayerCellId":2,"allowSubsetChampionPicks":true,"benchChampions":[{"championId":67}]});
        let mut first = selection(&session).unwrap();
        assert_eq!(first.local_slot, Some(1));
        assert_eq!(first.choices, Some([0; 3]));
        first.choices = subset(b"[875,103]");
        assert_eq!(first.choices, Some([875, 103, 0]));
        let second = selection(&session).unwrap();
        assert_eq!(second.choices, Some([0; 3]));
        assert_eq!(second.bench[0], 67);
        session["myTeam"][0]["championId"] = 103.into();
        let completed = selection(&session).unwrap();
        assert_eq!(completed.choices, None);
        assert_eq!(completed.team[1], 103);
        assert_eq!(completed.bench[0], 67);
        session["myTeam"][0]["championId"] = 0.into();
        assert_eq!(selection(&session).unwrap().choices, Some([0; 3]));
        session["allowSubsetChampionPicks"] = false.into();
        assert_eq!(selection(&session).unwrap().choices, None);
    }

    use super::*;
    #[test]
    fn bench_refresh_is_independent_of_local_pick_and_clears_disabled_pool() {
        let flow = br#"{"phase":"ChampSelect","gameData":{"queue":{"id":2400}}}"#;
        let body = serde_json::json!({"localPlayerCellId":2,"myTeam":[{"cellId":2,"championId":8},{"cellId":1,"championId":9}],
            "benchEnabled":true,"benchChampions":[{"championId":10},{"championId":10},{"championId":8}]});
        let a = parse(
            Phase::ChampSelect,
            flow,
            &serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();
        let s = a.selection.unwrap();
        assert_eq!(s.team, [9, 8, 0, 0, 0]);
        assert_eq!(s.local_slot, Some(1));
        assert_eq!(s.count, 1);
        let mut changed = body.clone();
        changed["benchChampions"] = serde_json::json!([{"championId":11}]);
        let b = parse(
            Phase::ChampSelect,
            flow,
            &serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert_eq!(a.champion, b.champion);
        assert!(a != b);
        assert_eq!(b.selection.unwrap().bench[0], 11);
        changed["benchEnabled"] = false.into();
        assert_eq!(
            parse(
                Phase::ChampSelect,
                flow,
                &serde_json::to_vec(&changed).unwrap()
            )
            .unwrap()
            .selection
            .unwrap()
            .count,
            0
        );
        changed["myTeam"][1]["cellId"] = 2.into();
        assert!(parse(
            Phase::ChampSelect,
            flow,
            &serde_json::to_vec(&changed).unwrap()
        )
        .is_err());
    }
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
