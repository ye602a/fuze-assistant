//! Client-order bench slots. A grab is always addressed to a champion, never a slot.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lobby {
    pub slots: [u32; 10],
    pub game: u64,
    pub ux_pid: u32,
}
impl Lobby {
    pub(crate) fn parse(session: &Value, game: u64) -> Option<Self> {
        if session["benchEnabled"] != true
            || matches!(
                session["timer"]["phase"].as_str(),
                Some("GAME_STARTING" | "PLANNING")
            )
        {
            return None;
        }
        let rows = session["benchChampions"].as_array()?;
        if rows.len() > 10 {
            return None;
        }
        let team = session["myTeam"].as_array()?;
        let cell = session["localPlayerCellId"].as_u64()?;
        if !team.iter().any(|p| {
            p["cellId"].as_u64() == Some(cell) && p["championId"].as_u64().is_some_and(|id| id > 0)
        }) {
            return None;
        }
        let mut slots = [0; 10];
        for (slot, row) in slots.iter_mut().zip(rows) {
            if let Some(id) = row["championId"]
                .as_u64()
                .and_then(|id| u32::try_from(id).ok())
            {
                if !team
                    .iter()
                    .any(|p| p["championId"].as_u64() == Some(id.into()))
                {
                    *slot = id;
                }
            }
        }
        // A click can outlive its original lobby while the client is reconnecting.
        let game = session["gameId"]
            .as_u64()
            .filter(|id| *id > 0)
            .unwrap_or(game);
        (game != 0).then_some(Self {
            slots,
            ux_pid: 0,
            game,
        })
    }
    pub fn offers(&self, game: u64, hero: u32) -> bool {
        self.game == game && hero != 0 && self.slots.contains(&hero)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attempt {
    Accepted,
    Stop,
    Retry,
    Backoff,
}

/// Only the bench write uses this policy; other LCU endpoints keep their own status contract.
pub(crate) fn response(code: u32) -> crate::Result<Attempt> {
    match code {
        200..=299 => Ok(Attempt::Accepted),
        400 | 409 => Ok(Attempt::Retry),
        429 | 500 | 502 | 503 | 504 => Ok(Attempt::Backoff),
        _ => crate::status(code).map(|_| Attempt::Accepted),
    }
}

pub(crate) fn attempt(
    lobby: Option<Lobby>,
    game: u64,
    hero: u32,
    active: &std::sync::atomic::AtomicBool,
    post: impl FnOnce(u32) -> crate::Result<Attempt>,
) -> crate::Result<Attempt> {
    if active.load(std::sync::atomic::Ordering::Acquire)
        && lobby.is_some_and(|l| l.offers(game, hero))
    {
        post(hero)
    } else {
        Ok(Attempt::Stop)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::parse;
    use fuze_runtime::session::Phase;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn bench_status_distinguishes_waiting_from_fatal_rejection() {
        for code in [400, 409] {
            assert_eq!(response(code), Ok(Attempt::Retry));
        }
        for code in [429, 500, 502, 503, 504] {
            assert_eq!(response(code), Ok(Attempt::Backoff));
        }
        for code in [200, 204] {
            assert_eq!(response(code), Ok(Attempt::Accepted));
        }
        for code in [401, 403, 404, 405, 422, 501] {
            assert!(response(code).is_err());
        }
    }
    #[test]
    fn locked_target_retries_until_swap_is_accepted_and_never_switches_slots() {
        let session = json!({"localPlayerCellId":1,"myTeam":[{"cellId":1,"championId":2}],"benchEnabled":true,"benchChampions":[{"championId":22},{"championId":11}]});
        let mut lobby = Lobby::parse(&session, 81).unwrap();
        let active = AtomicBool::new(true);
        let mut requests = Vec::new();
        // The endpoint rejects swaps during the portrait cooldown, then accepts.
        for response in [Attempt::Retry, Attempt::Backoff, Attempt::Accepted] {
            assert!(lobby.offers(81, 22));
            assert_eq!(
                attempt(Some(lobby), 81, 22, &active, |hero| {
                    requests.push(hero);
                    Ok(response)
                }),
                Ok(response)
            );
            lobby.slots.swap(0, 1);
        }
        assert_eq!(requests, [22, 22, 22]);
        active.store(false, Ordering::Release);
        assert_eq!(
            attempt(Some(lobby), 81, 22, &active, |_| panic!("cancelled")),
            Ok(Attempt::Stop)
        );
        active.store(true, Ordering::Release);
        for slot in &mut lobby.slots {
            if *slot == 22 {
                *slot = 99;
            }
        }
        assert_eq!(
            attempt(Some(lobby), 81, 22, &active, |_| panic!("replacement hero")),
            Ok(Attempt::Stop)
        );
    }

    #[test]
    fn grab_stays_on_clicked_champion_across_reorders_and_terminates() {
        let flow = br#"{"phase":"ChampSelect","gameData":{"queue":{"id":2400},"gameId":81}}"#;
        let mut session = json!({"localPlayerCellId":1,"myTeam":[{"cellId":1,"championId":2},{"cellId":2,"championId":3}],"benchEnabled":true,"benchChampions":[{"championId":0},{"championId":22},{"championId":11}]});
        let lobby = |s: &Value| {
            parse(Phase::ChampSelect, flow, &serde_json::to_vec(s).unwrap())
                .unwrap()
                .bench_lobby
        };
        assert!(Lobby::parse(&session, 0).is_none());
        let first = lobby(&session).unwrap();
        assert_eq!(first.slots[..3], [0, 22, 11]);
        let hero = first.slots[1];
        let active = AtomicBool::new(true);
        let mut sent = Vec::new();
        assert_eq!(
            attempt(Some(first), 81, hero, &active, |id| {
                sent.push(id);
                Ok(Attempt::Retry)
            }),
            Ok(Attempt::Retry)
        );
        session["benchChampions"] = json!([{"championId":22},{"championId":99}]);
        assert_eq!(
            attempt(lobby(&session), 81, hero, &active, |id| {
                sent.push(id);
                Ok(Attempt::Accepted)
            }),
            Ok(Attempt::Accepted)
        );
        assert_eq!(sent, [22, 22]);

        let stopped = |l| {
            assert_eq!(
                attempt(l, 81, hero, &active, |_| panic!("unexpected swap")),
                Ok(Attempt::Stop)
            )
        };
        active.store(false, Ordering::Release);
        stopped(lobby(&session)); // F9 while waiting for a fresh read
        active.store(true, Ordering::Release);
        for player in [0, 1] {
            let mut changed = session.clone();
            changed["myTeam"][player]["championId"] = hero.into();
            stopped(lobby(&changed)); // local success or teammate got there first
        }
        let mut gone = session.clone();
        gone["benchChampions"] = json!([{"championId":99}]);
        stopped(lobby(&gone));
        session["gameId"] = 82.into();
        stopped(lobby(&session));
        stopped(None); // left the lobby
        session["benchEnabled"] = false.into();
        assert!(lobby(&session).is_none());
    }
}
