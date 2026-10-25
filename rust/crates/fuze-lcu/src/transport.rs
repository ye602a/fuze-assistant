//! Fixed loopback HTTPS. Reads by default; the explicit item-set transaction alone permits PUT.
//! Temporary identity responses are reduced to local hero context, never logged or returned.
use super::{basic, parse_phase, status, wipe, Credentials, Endpoint, Error, Result};
use fuze_runtime::session::Phase;
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinHttp::*;
#[path = "sgp.rs"]
mod sgp;
struct Handle(*mut c_void);
impl Handle {
    fn new(p: *mut c_void) -> Result<Self> {
        if p.is_null() {
            Err(Error::Transport)
        } else {
            Ok(Self(p))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn ok(value: i32) -> Result<()> {
    if value == 0 {
        Err(Error::Transport)
    } else {
        Ok(())
    }
}
fn option(handle: &Handle, name: u32, value: u32) -> Result<()> {
    ok(unsafe { WinHttpSetOption(handle.0, name, (&value as *const u32).cast(), 4) })
}
pub(crate) struct Header(pub(crate) Vec<u16>);
impl Header {
    pub(crate) fn new(c: &Credentials) -> Self {
        let mut encoded = basic(&c.secret);
        let mut header: Vec<u16> = "Authorization: Basic ".encode_utf16().collect();
        header.extend(encoded.iter().map(|b| *b as u16));
        header.extend("\r\n".encode_utf16());
        wipe(&mut encoded);
        Self(header)
    }
}
impl Drop for Header {
    fn drop(&mut self) {
        for word in &mut self.0 {
            unsafe { std::ptr::write_volatile(word, 0) }
        }
    }
}
/// Run on an owned background IO worker, never UI/OCR threads. WinHTTP timeouts are per-operation,
/// not a hard cancellation deadline; the future event-loop host must provide process supervision.
pub struct ReadOnlyClient {
    connection: Handle,
    _session: Handle,
    credentials: Credentials,
}
impl ReadOnlyClient {
    pub(crate) fn journal_owner(
        &self,
        pid: u32,
    ) -> crate::history::Result<(crate::history::Owner, u64)> {
        let before = self.context(pid).map_err(|_| "journal-context")?;
        let game = before.observed_game_id().ok_or("journal-context")?;
        let summoner = self.history_value(pid, Endpoint::SummonerIdentity)?;
        let mut routing = Vec::new();
        for endpoint in [Endpoint::Authorization, Endpoint::ChatMe] {
            if let Ok(v) = self.history_value(pid, endpoint) {
                routing.push(v);
            }
        }
        let owner = crate::history::Owner::from_values(&summoner, &routing)?;
        let after = self.context(pid).map_err(|_| "journal-context")?;
        if before != after {
            return Err("journal-context-changed");
        }
        let verified = self.history_value(pid, Endpoint::SummonerIdentity)?;
        if crate::history::Owner::from_values(&verified, &routing)? != owner {
            return Err("journal-account-changed");
        }
        Ok((owner, game))
    }
    fn history_value(
        &self,
        pid: u32,
        endpoint: Endpoint,
    ) -> crate::history::Result<serde_json::Value> {
        let (_, mut body) = self.request(pid, endpoint).map_err(|e| match e {
            Error::Unauthorized => "history-auth-failed",
            Error::NotFound => "history-endpoint-unavailable",
            Error::ResponseLimit => "history-response-limit",
            _ => "history-request-failed",
        })?;
        let parsed = serde_json::from_slice(&body).map_err(|_| "history-invalid-response");
        wipe(&mut body);
        parsed
    }
    pub(crate) fn history_owner(&self, pid: u32) -> crate::history::Result<crate::history::Owner> {
        let phase = self.history_value(pid, Endpoint::Phase)?;
        if !matches!(
            phase.as_str(),
            Some("None" | "Lobby" | "WaitingForStats" | "PreEndOfGame" | "EndOfGame")
        ) {
            return Err("history-idle-required");
        }
        let summoner = self.history_value(pid, Endpoint::SummonerIdentity)?;
        let mut routing = Vec::new();
        for endpoint in [Endpoint::Authorization, Endpoint::ChatMe] {
            if let Ok(value) = self.history_value(pid, endpoint) {
                routing.push(value);
            }
        }
        crate::history::Owner::from_values(&summoner, &routing)
    }
    pub(crate) fn history_rows(
        &self,
        pid: u32,
        owner: &crate::history::Owner,
        start: u32,
    ) -> crate::history::Result<Vec<crate::history::Match>> {
        if start > 950 || start % 50 != 0 {
            return Err("history-page-limit");
        }
        let local=if start==0 {self.history_value(pid,Endpoint::History)}else{
            let path=format!("/lol-match-history/v1/products/lol/current-summoner/matches?begIndex={start}&endIndex={}",start+49);
            self.request_at(pid,&path,"GET",&[],4*1024*1024,false).map_err(|_|"history-request-failed").and_then(|(_,mut bytes)|{let result=serde_json::from_slice(&bytes).map_err(|_|"history-invalid-response");wipe(&mut bytes);result})
        }.and_then(|v|crate::history::parse(owner,&v));
        local
    }
    pub(crate) fn history_sgp(
        &self,
        pid: u32,
        owner: &crate::history::Owner,
        start: u32,
    ) -> crate::history::Result<Vec<crate::history::Match>> {
        if start > 950 || start % 50 != 0 {
            return Err("history-page-limit");
        }
        if &self.history_owner(pid)? != owner {
            return Err("history-account-changed");
        }
        let token = self.history_value(pid, Endpoint::Entitlements)?;
        let result = sgp::fetch(owner, &token, start);
        if &self.history_owner(pid)? != owner {
            return Err("history-account-changed");
        }
        result
    }
    pub(crate) fn install_item_sets(
        &self,
        pid: u32,
        champion: u32,
        sets: &[serde_json::Value],
    ) -> crate::history::Result<()> {
        let verify = || -> crate::history::Result<(u64, u64)> {
            let c = self
                .context(pid)
                .map_err(|_| "item-sets-client-unavailable")?;
            if c.phase != Phase::ChampSelect || c.champion != Some(champion) {
                return Err("item-sets-selection-changed");
            }
            let s = self.history_value(pid, Endpoint::SummonerIdentity)?;
            let id = |key: &str| {
                s[key]
                    .as_u64()
                    .filter(|n| *n > 0 && *n <= 9_007_199_254_740_991)
                    .ok_or("item-sets-account-unavailable")
            };
            Ok((id("summonerId")?, id("accountId")?))
        };
        let identity = verify()?;
        let (summoner, account) = identity;
        let path = format!("/lol-item-sets/v1/item-sets/{summoner}/sets");
        let (_, mut bytes) = self
            .request_at(pid, &path, "GET", &[], 1024 * 1024, false)
            .map_err(|_| "item-sets-read-failed")?;
        let current = serde_json::from_slice(&bytes).map_err(|_| "item-sets-current-invalid");
        wipe(&mut bytes);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "item-sets-clock")?
            .as_millis() as u64;
        let merged = crate::item_sets::merge(&current?, sets, champion, account, now)?;
        if verify()? != identity {
            return Err("item-sets-account-changed");
        }
        let mut bytes = serde_json::to_vec(&merged).map_err(|_| "item-sets-invalid")?;
        let result = self
            .request_at(pid, &path, "PUT", &bytes, 0, true)
            .map(|_| ())
            .map_err(|_| "item-sets-write-failed");
        wipe(&mut bytes);
        result?;
        let (_, mut saved) = self
            .request_at(pid, &path, "GET", &[], 1024 * 1024, false)
            .map_err(|_| "item-sets-readback-failed")?;
        let decoded = serde_json::from_slice(&saved).map_err(|_| "item-sets-readback-failed");
        wipe(&mut saved);
        crate::item_sets::verify_saved(&merged, &decoded?)
    }
    pub fn websocket(&self) -> Result<crate::websocket::Socket> {
        crate::websocket::Socket::connect(&self.credentials)
    }
    pub fn locale(&self, pid: u32) -> Result<crate::locale::Locale> {
        let (_, mut body) = self.request(pid, Endpoint::Locale)?;
        let result = crate::locale::parse(&body);
        wipe(&mut body);
        result
    }
    pub fn context(&self, current_pid: u32) -> Result<crate::context::Context> {
        let phase = self.phase(current_pid)?;
        if !matches!(
            phase,
            Phase::ChampSelect | Phase::Loading | Phase::InProgress
        ) {
            return Ok(crate::context::Context::idle(phase));
        }
        let (_, mut flow) = self.request(current_pid, Endpoint::Gameflow)?;
        let endpoint = if phase == Phase::ChampSelect {
            Endpoint::ChampSelect
        } else {
            Endpoint::SummonerIdentity
        };
        let result = (|| {
            let (_, mut player) = self.request(current_pid, endpoint)?;
            let result = crate::context::parse(phase, &flow, &player);
            wipe(&mut player);
            let mut context = result?;
            if let Some(selection) = context.selection.as_mut().filter(|s| s.choices.is_some()) {
                if let Ok((code, mut body)) = self.request(current_pid, Endpoint::SubsetChampions) {
                    selection.choices = Some(if code == 200 {
                        crate::context::subset(&body).unwrap_or([0; 3])
                    } else {
                        [0; 3]
                    });
                    wipe(&mut body);
                }
            }
            Ok(context)
        })();
        wipe(&mut flow);
        result
    }
    pub fn new(credentials: Credentials) -> Result<Self> {
        unsafe {
            let session = Handle::new(WinHttpOpen(
                wide("Fuze-Rust-local").as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                null(),
                null(),
                0,
            ))?;
            ok(WinHttpSetTimeouts(session.0, 500, 500, 500, 500))?;
            let connection = Handle::new(WinHttpConnect(
                session.0,
                wide("127.0.0.1").as_ptr(),
                credentials.port,
                0,
            ))?;
            Ok(Self {
                connection,
                _session: session,
                credentials,
            })
        }
    }
    pub fn phase(&self, current_pid: u32) -> Result<Phase> {
        let (status, body) = self.request(current_pid, Endpoint::Phase)?;
        parse_phase(status, &body)
    }
    pub fn summoner_status(&self, current_pid: u32) -> Result<()> {
        let (code, _) = self.request(current_pid, Endpoint::SummonerStatus)?;
        status(code)
    }
    fn request(&self, current_pid: u32, endpoint: Endpoint) -> Result<(u32, Vec<u8>)> {
        let limit = if matches!(endpoint, Endpoint::Phase) {
            128
        } else if matches!(endpoint, Endpoint::History) {
            4 * 1024 * 1024
        } else {
            256 * 1024
        };
        self.request_at(
            current_pid,
            endpoint.path(),
            "GET",
            &[],
            limit,
            matches!(endpoint, Endpoint::SummonerStatus),
        )
    }
    // Private transport: only typed read endpoints and the explicit item-set transaction reach here.
    fn request_at(
        &self,
        current_pid: u32,
        path: &str,
        method: &str,
        payload: &[u8],
        limit: usize,
        status_only: bool,
    ) -> Result<(u32, Vec<u8>)> {
        if !self.credentials.belongs_to(current_pid) {
            return Err(Error::AuthInput);
        }
        unsafe {
            let request = Handle::new(WinHttpOpenRequest(
                self.connection.0,
                wide(method).as_ptr(),
                wide(path).as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            option(
                &request,
                WINHTTP_OPTION_REDIRECT_POLICY,
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
            )?;
            option(
                &request,
                WINHTTP_OPTION_DISABLE_FEATURE,
                WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION,
            )?;
            // LCU's local certificate exception is confined to this fixed loopback request.
            option(
                &request,
                WINHTTP_OPTION_SECURITY_FLAGS,
                SECURITY_FLAG_IGNORE_UNKNOWN_CA | SECURITY_FLAG_IGNORE_CERT_CN_INVALID,
            )?;
            let mut header = Header::new(&self.credentials);
            if method == "PUT" {
                header
                    .0
                    .extend("Content-Type: application/json\r\n".encode_utf16());
            }
            ok(WinHttpSendRequest(
                request.0,
                header.0.as_ptr(),
                header.0.len() as u32,
                if payload.is_empty() {
                    null()
                } else {
                    payload.as_ptr().cast()
                },
                payload.len() as u32,
                payload.len() as u32,
                0,
            ))?;
            drop(header);
            ok(WinHttpReceiveResponse(request.0, null_mut()))?;
            let mut code = 0u32;
            let mut size = 4u32;
            ok(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut code as *mut u32).cast(),
                &mut size,
                null_mut(),
            ))?;
            if !(method == "PUT" && (200..300).contains(&code)) {
                status(code)?;
            }
            // Deliberately never read the body of status-only probes or successful writes.
            if status_only {
                return Ok((code, Vec::new()));
            }
            let started = Instant::now();
            let mut body = Vec::with_capacity(limit.min(4096));
            for _ in 0..=limit.div_ceil(4096) {
                if started.elapsed() > Duration::from_secs(2) {
                    return Err(Error::Transport);
                }
                let mut buffer = [0u8; 4096];
                let mut read = 0u32;
                ok(WinHttpReadData(
                    request.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut read,
                ))?;
                if read == 0 {
                    return Ok((code, body));
                }
                if body.len() + read as usize > limit {
                    return Err(Error::ResponseLimit);
                }
                body.extend_from_slice(&buffer[..read as usize]);
            }
            Err(Error::ResponseLimit)
        }
    }
}

/// Live Client Data uses a separate fixed loopback endpoint, never LCU credentials.
/// Only a level survives parsing; no identities are materialized or returned.
pub fn active_player_level() -> Result<u8> {
    let mut body = live_probe(1)?;
    let result = parse_active_player_level(&body);
    wipe(&mut body);
    result
}
#[derive(Clone, Copy)]
pub struct LiveState {
    pub level: u8,
    pub dead: Option<bool>,
}
/// Read-only Live Client Data. Identities are borrowed only to match the active
/// player in-process; only level/dead leave this function, and response buffers wipe.
pub fn active_player_state() -> Result<LiveState> {
    let mut active = live_probe(1)?;
    let level = parse_active_player_level(&active);
    let mut players = if level.is_ok() {
        live_probe(3).ok()
    } else {
        None
    };
    let dead = players.as_ref().and_then(|p| parse_active_dead(&active, p));
    wipe(&mut active);
    if let Some(p) = &mut players {
        wipe(p);
    }
    level.map(|level| LiveState { level, dead })
}
fn parse_active_dead(active: &[u8], players: &[u8]) -> Option<bool> {
    #[derive(serde::Deserialize)]
    struct Active<'a> {
        #[serde(rename = "riotId", borrow)]
        id: &'a str,
    }
    #[derive(serde::Deserialize)]
    struct Player<'a> {
        #[serde(rename = "riotId", borrow)]
        id: &'a str,
        #[serde(rename = "isDead")]
        dead: bool,
    }
    if active.len() > 65536 || players.len() > 256 * 1024 {
        return None;
    }
    let a: Active = serde_json::from_slice(active).ok()?;
    if a.id.is_empty() {
        return None;
    }
    let p: Vec<Player> = serde_json::from_slice(players).ok()?;
    if p.len() > 64 {
        return None;
    }
    let mut matches = p.iter().filter(|p| p.id == a.id);
    let dead = matches.next()?.dead;
    matches.next().is_none().then_some(dead)
}
pub(crate) fn live_probe(index: usize) -> Result<Vec<u8>> {
    let (path, limit) = match index {
        0 => ("/swagger/v3/openapi.json", 2 * 1024 * 1024),
        1 => ("/liveclientdata/activeplayer", 256 * 1024),
        2 => ("/liveclientdata/eventdata", 256 * 1024),
        3 => ("/liveclientdata/playerlist", 256 * 1024),
        _ => return Err(Error::AuthInput),
    };
    struct Body(Vec<u8>);
    impl Drop for Body {
        fn drop(&mut self) {
            wipe(&mut self.0);
        }
    }
    unsafe {
        let session = Handle::new(WinHttpOpen(
            wide("Fuze-level-local").as_ptr(),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            null(),
            null(),
            0,
        ))?;
        ok(WinHttpSetTimeouts(session.0, 300, 300, 300, 300))?;
        let connection = Handle::new(WinHttpConnect(
            session.0,
            wide("127.0.0.1").as_ptr(),
            2999,
            0,
        ))?;
        let request = Handle::new(WinHttpOpenRequest(
            connection.0,
            wide("GET").as_ptr(),
            wide(path).as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        option(
            &request,
            WINHTTP_OPTION_REDIRECT_POLICY,
            WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
        )?;
        option(
            &request,
            WINHTTP_OPTION_DISABLE_FEATURE,
            WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION,
        )?;
        option(
            &request,
            WINHTTP_OPTION_SECURITY_FLAGS,
            SECURITY_FLAG_IGNORE_UNKNOWN_CA | SECURITY_FLAG_IGNORE_CERT_CN_INVALID,
        )?;
        ok(WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0))?;
        ok(WinHttpReceiveResponse(request.0, null_mut()))?;
        let mut code = 0u32;
        let mut size = 4u32;
        ok(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut code as *mut u32).cast(),
            &mut size,
            null_mut(),
        ))?;
        if code != 200 {
            return Err(Error::HttpStatus);
        }
        let started = Instant::now();
        let mut body = Body(Vec::with_capacity(4096));
        for _ in 0..(limit / 4096 + 1) {
            if started.elapsed() > Duration::from_millis(900) {
                return Err(Error::Transport);
            }
            let mut buffer = Body(vec![0; 4096]);
            let mut read = 0;
            ok(WinHttpReadData(
                request.0,
                buffer.0.as_mut_ptr().cast(),
                4096,
                &mut read,
            ))?;
            if read == 0 {
                return Ok(std::mem::take(&mut body.0));
            }
            if body.0.len() + read as usize > limit {
                return Err(Error::ResponseLimit);
            }
            body.0.extend_from_slice(&buffer.0[..read as usize]);
        }
        Err(Error::ResponseLimit)
    }
}

pub fn parse_active_player_level(body: &[u8]) -> Result<u8> {
    #[derive(serde::Deserialize)]
    struct Level {
        level: u8,
    }
    if body.len() > 65536 {
        return Err(Error::ResponseLimit);
    }
    let value: Level = serde_json::from_slice(body).map_err(|_| Error::HttpStatus)?;
    (1..=30)
        .contains(&value.level)
        .then_some(value.level)
        .ok_or(Error::HttpStatus)
}
#[cfg(test)]
mod level_tests {
    use super::*;
    #[test]
    fn death_signal_requires_one_exact_active_player_and_never_guesses_from_team() {
        let a = br##"{"riotId":"fixture#one","level":8}"##;
        assert_eq!(parse_active_dead(a,br##"[{"riotId":"fixture#other","isDead":true},{"riotId":"fixture#one","isDead":false}]"##),Some(false));
        assert_eq!(
            parse_active_dead(a, br##"[{"riotId":"fixture#one","isDead":true}]"##),
            Some(true)
        );
        assert_eq!(
            parse_active_dead(a, br##"[{"riotId":"fixture#other","isDead":true}]"##),
            None
        );
        assert_eq!(parse_active_dead(a,br##"[{"riotId":"fixture#one","isDead":true},{"riotId":"fixture#one","isDead":false}]"##),None);
        assert_eq!(parse_active_dead(br##"{"level":8}"##, b"[]"), None);
    }
    #[test]
    fn only_integer_level_survives_untrusted_response() {
        assert_eq!(
            parse_active_player_level(
                br#"{"level":7,"summonerName":"fixture-private","nested":{"identity":"discard"}}"#
            ),
            Ok(7)
        );
        for body in [
            br#"{"level":0}"#.as_slice(),
            br#"{"level":31}"#,
            br#"{"level":"7"}"#,
            br#"{"level":7.5}"#,
            br#"{}"#,
            br#"{"level":3,"level":7}"#,
        ] {
            assert!(parse_active_player_level(body).is_err());
        }
        assert_eq!(
            parse_active_player_level(&vec![b' '; 65537]),
            Err(Error::ResponseLimit)
        );
    }
}
