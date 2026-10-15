//! Fixed loopback HTTPS only. No arbitrary URL, proxy, redirect or write method.
//! Temporary identity responses are reduced to local hero context, never logged or returned.
use super::{basic, parse_phase, status, wipe, Credentials, Endpoint, Error, Result};
use fuze_runtime::session::Phase;
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinHttp::*;
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
struct Header(Vec<u16>);
impl Header {
    fn new(c: &Credentials) -> Self {
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
            result
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
        if !self.credentials.belongs_to(current_pid) {
            return Err(Error::AuthInput);
        }
        unsafe {
            let request = Handle::new(WinHttpOpenRequest(
                self.connection.0,
                wide("GET").as_ptr(),
                wide(endpoint.path()).as_ptr(),
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
            let header = Header::new(&self.credentials);
            ok(WinHttpSendRequest(
                request.0,
                header.0.as_ptr(),
                header.0.len() as u32,
                null(),
                0,
                0,
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
            status(code)?;
            // Deliberately never read, parse, retain or log the summoner identity response.
            if matches!(endpoint, Endpoint::SummonerStatus) {
                return Ok((code, Vec::new()));
            }
            let started = Instant::now();
            let limit = if matches!(endpoint, Endpoint::Phase) {
                128
            } else {
                256 * 1024
            };
            let mut body = Vec::with_capacity(limit);
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
