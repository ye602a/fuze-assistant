//! Read-only LCU boundary with a separately invoked, player-requested item-set transaction. Nothing runs at import; caller supplies freshly observed credentials.
use fuze_runtime::session::Phase;
use std::fmt;
#[cfg(windows)]
mod client_log;
#[cfg(windows)]
mod command_line;
pub mod context;
#[cfg(windows)]
mod directory_auth;
#[cfg(windows)]
pub mod discovery;
pub mod events;
pub mod history;
pub mod install;
pub mod item_sets;
pub mod locale;
#[cfg(windows)]
mod process_watch;
pub mod reconnect;
pub mod stage_probe;
#[cfg(windows)]
pub mod transport;
#[cfg(windows)]
pub mod websocket;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Error {
    AuthInput,
    Discovery,
    AmbiguousClient,
    Unauthorized,
    NotFound,
    HttpStatus,
    InvalidPhase,
    ResponseLimit,
    Transport,
}
pub type Result<T> = std::result::Result<T, Error>;
/// Deliberately not Clone/Serialize; Debug never exposes port, password, arguments or identity.
pub struct Credentials {
    pid: u32,
    port: u16,
    secret: Vec<u8>,
}
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials([redacted])")
    }
}
impl Drop for Credentials {
    fn drop(&mut self) {
        wipe(&mut self.secret);
    }
}
pub(crate) fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe { std::ptr::write_volatile(byte, 0) }
    }
}
impl Credentials {
    fn new(pid: u32, port: u16, secret: &str) -> Result<Self> {
        if pid == 0
            || port == 0
            || secret.is_empty()
            || secret.len() > 256
            || !secret.bytes().all(|b| b.is_ascii_graphic() && b != b':')
        {
            return Err(Error::AuthInput);
        }
        Ok(Self {
            pid,
            port,
            secret: secret.as_bytes().to_vec(),
        })
    }
    /// Inputs must come from the process-first discovery adapter, not a renderer or remote config.
    pub fn from_process_args(pid: u32, image_name: &str, args: &[String]) -> Result<Self> {
        if !image_name.eq_ignore_ascii_case("LeagueClientUx.exe") || args.len() > 256 {
            return Err(Error::AuthInput);
        }
        let mut port = None;
        let mut secret = None;
        for arg in args {
            if arg.len() > 32768 {
                return Err(Error::AuthInput);
            }
            if let Some(value) = arg.strip_prefix("--app-port=") {
                if port.is_some() {
                    return Err(Error::AuthInput);
                }
                port = Some(value.parse::<u16>().map_err(|_| Error::AuthInput)?);
            }
            if let Some(value) = arg.strip_prefix("--remoting-auth-token=") {
                if secret.is_some() {
                    return Err(Error::AuthInput);
                }
                secret = Some(value);
            }
        }
        Self::new(
            pid,
            port.ok_or(Error::AuthInput)?,
            secret.ok_or(Error::AuthInput)?,
        )
    }
    /// Advanced fallback only. Caller must verify this PID is the currently observed client.
    pub fn from_lockfile(text: &str, observed_pid: u32) -> Result<Self> {
        if text.len() > 1024 {
            return Err(Error::AuthInput);
        }
        let fields: Vec<_> = text.trim_end_matches(['\r', '\n']).split(':').collect();
        if fields.len() != 5 || fields[0] != "LeagueClient" || fields[4] != "https" {
            return Err(Error::AuthInput);
        }
        let pid = fields[1].parse::<u32>().map_err(|_| Error::AuthInput)?;
        if pid != observed_pid {
            return Err(Error::AuthInput);
        }
        Self::new(
            pid,
            fields[2].parse::<u16>().map_err(|_| Error::AuthInput)?,
            fields[3],
        )
    }
    pub fn belongs_to(&self, current_pid: u32) -> bool {
        self.pid == current_pid
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Endpoint {
    Phase,
    SummonerStatus,
    SummonerIdentity,
    Gameflow,
    ChampSelect,
    Locale,
    History,
    Authorization,
    ChatMe,
    Entitlements,
}
impl Endpoint {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Phase => "/lol-gameflow/v1/gameflow-phase",
            Self::SummonerStatus | Self::SummonerIdentity => "/lol-summoner/v1/current-summoner",
            Self::Gameflow => "/lol-gameflow/v1/session",
            Self::ChampSelect => "/lol-champ-select/v1/session",
            Self::Locale => "/riotclient/region-locale",
            Self::History => {
                "/lol-match-history/v1/products/lol/current-summoner/matches?begIndex=0&endIndex=49"
            }
            Self::Authorization => "/lol-rso-auth/v1/authorization",
            Self::ChatMe => "/lol-chat/v1/me",
            Self::Entitlements => "/entitlements/v1/token",
        }
    }
}
pub fn status(code: u32) -> Result<()> {
    match code {
        200 => Ok(()),
        401 | 403 => Err(Error::Unauthorized),
        404 => Err(Error::NotFound),
        _ => Err(Error::HttpStatus),
    }
}
pub fn parse_phase(code: u32, body: &[u8]) -> Result<Phase> {
    status(code)?;
    if body.len() > 128 {
        return Err(Error::ResponseLimit);
    }
    let value: String = serde_json::from_slice(body).map_err(|_| Error::InvalidPhase)?;
    let phase = Phase::from_lcu(Some(&value));
    if phase == Phase::Disconnected {
        Err(Error::InvalidPhase)
    } else {
        Ok(phase)
    }
}
/// Parse only a gameflow subscription event from an already authenticated WebSocket.
/// This parser alone never proves a connected socket or successful authentication.
pub fn parse_gameflow_event(body: &[u8]) -> Result<Option<Phase>> {
    if body.len() > 32768 {
        return Err(Error::ResponseLimit);
    }
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|_| Error::InvalidPhase)?;
    let Some(parts) = value.as_array() else {
        return Err(Error::InvalidPhase);
    };
    if parts.len() != 3
        || parts[0].as_u64() != Some(8)
        || parts[1].as_str() != Some("OnJsonApiEvent")
    {
        return Ok(None);
    }
    let event = &parts[2];
    if event.get("uri").and_then(|v| v.as_str()) != Some(Endpoint::Phase.path()) {
        return Ok(None);
    }
    if event.get("eventType").and_then(|v| v.as_str()) == Some("Delete") {
        return Ok(Some(Phase::Disconnected));
    }
    if !matches!(
        event.get("eventType").and_then(|v| v.as_str()),
        Some("Update" | "Create")
    ) {
        return Ok(None);
    }
    let data = serde_json::to_vec(event.get("data").ok_or(Error::InvalidPhase)?)
        .map_err(|_| Error::InvalidPhase)?;
    parse_phase(200, &data).map(Some)
}
pub(crate) fn basic(secret: &[u8]) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut raw = b"riot:".to_vec();
    raw.extend_from_slice(secret);
    let mut out = Vec::with_capacity(raw.len().div_ceil(3) * 4);
    for chunk in raw.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        out.push(TABLE[a >> 2]);
        out.push(TABLE[((a & 3) << 4) | (b >> 4)]);
        out.push(if chunk.len() > 1 {
            TABLE[((b & 15) << 2) | (c >> 6)]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 { TABLE[c & 63] } else { b'=' });
    }
    wipe(&mut raw);
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_redacted_bound_to_process_and_duplicate_flags_fail() {
        let c = Credentials::from_lockfile("LeagueClient:12:12345:fixture:https", 12).unwrap();
        assert_eq!(format!("{c:?}"), "Credentials([redacted])");
        assert!(!c.belongs_to(13));
        assert!(Credentials::from_lockfile("LeagueClient:12:12345:fixture:http", 12).is_err());
        assert!(Credentials::from_lockfile("LeagueClient:12:12345:fixture:https", 13).is_err());
        let args = vec![
            "--app-port=12345".into(),
            "--remoting-auth-token=fixture".into(),
        ];
        assert!(Credentials::from_process_args(12, "Other.exe", &args).is_err());
        assert!(Credentials::from_process_args(12, "LeagueClientUx.exe", &args).is_ok());
        let mut duplicate = args;
        duplicate.push("--app-port=9999".into());
        assert!(Credentials::from_process_args(12, "LeagueClientUx.exe", &duplicate).is_err());
        assert_eq!(basic(b"test"), b"cmlvdDp0ZXN0");
    }
    #[test]
    fn only_successful_known_phase_is_usable_and_events_cannot_claim_rewards() {
        assert_eq!(parse_phase(200, br#""Lobby""#), Ok(Phase::ClientReady));
        assert_eq!(
            parse_phase(401, br#""InProgress""#),
            Err(Error::Unauthorized)
        );
        assert_eq!(parse_phase(404, br#""InProgress""#), Err(Error::NotFound));
        assert_eq!(
            parse_phase(200, br#""NewUnknownPhase""#),
            Err(Error::InvalidPhase)
        );
        assert_eq!(parse_gameflow_event(br#"[8,"OnJsonApiEvent",{"uri":"/lol-gameflow/v1/gameflow-phase","eventType":"Update","data":"InProgress"}]"#),Ok(Some(Phase::InProgress)));
        assert_eq!(parse_gameflow_event(br#"[8,"OnJsonApiEvent",{"uri":"/other","eventType":"Update","data":"InProgress"}]"#),Ok(None));
    }
}
