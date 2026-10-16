//! Remote downloads are separate from LCU: strict public TLS, no cookies/proxy/redirect.
use crate::Result;
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{Networking::WinHttp::*, Security::Cryptography::*};
struct Handle(*mut c_void);
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
fn handle(p: *mut c_void) -> Result<Handle> {
    if p.is_null() {
        Err("update-network")
    } else {
        Ok(Handle(p))
    }
}
fn network_error() -> &'static str {
    if unsafe { windows_sys::Win32::Foundation::GetLastError() } == ERROR_WINHTTP_TIMEOUT {
        "update-cancelled-or-timeout"
    } else {
        "update-network"
    }
}
pub fn digest(bytes: &[u8], sha512: bool) -> Result<Vec<u8>> {
    let mut result = vec![0; if sha512 { 64 } else { 32 }];
    if bytes.len() > u32::MAX as usize {
        return Err("update-size");
    }
    let status = unsafe {
        BCryptHash(
            if sha512 {
                BCRYPT_SHA512_ALG_HANDLE
            } else {
                BCRYPT_SHA256_ALG_HANDLE
            },
            null(),
            0,
            bytes.as_ptr(),
            bytes.len() as u32,
            result.as_mut_ptr(),
            result.len() as u32,
        )
    };
    if status < 0 {
        Err("update-hash")
    } else {
        Ok(result)
    }
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// No remote-provided URL is accepted. Resources are constructed from validated version/path.
pub fn data_get(path: &str, limit: usize, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    if !path.starts_with("/api/client/v1/")
        || path.len() > 512
        || path.chars().any(|c| c.is_control() || "\\#%@".contains(c))
    {
        return Err("update-url");
    }
    get("data.dtodo.cn", path, limit, None, None, cancel)
}
/// Callers select a fixed host and path through the private-channel policy, never LCU auth.
pub(crate) fn get(
    host: &str,
    path: &str,
    limit: usize,
    authorization: Option<&str>,
    range: Option<(u64, u64, u64)>,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    if transfer(host, path, limit, authorization, range, cancel, &mut output)?.is_some() {
        return Err("update-unexpected-redirect");
    }
    Ok(output)
}
/// Streams bytes into staging. Redirects are returned, never followed with ambient credentials.
pub(crate) fn transfer(
    host: &str,
    path: &str,
    limit: usize,
    authorization: Option<&str>,
    range: Option<(u64, u64, u64)>,
    cancel: &dyn Fn() -> bool,
    output: &mut dyn std::io::Write,
) -> Result<Option<String>> {
    let started = Instant::now();
    let check = || {
        if cancel()
            || started.elapsed()
                > Duration::from_secs(if limit > 8 * 1024 * 1024 { 600 } else { 45 })
        {
            Err("update-cancelled-or-timeout")
        } else {
            Ok(())
        }
    };
    check()?;
    unsafe {
        let session = handle(WinHttpOpen(
            wide("Fuze-Rust-update").as_ptr(),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            null(),
            null(),
            0,
        ))?;
        if WinHttpSetTimeouts(session.0, 5000, 5000, 10000, 20000) == 0 {
            return Err("update-network");
        }
        let connection = handle(WinHttpConnect(session.0, wide(host).as_ptr(), 443, 0))?;
        let request = handle(WinHttpOpenRequest(
            connection.0,
            wide("GET").as_ptr(),
            wide(path).as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        for (name, value) in [
            (
                WINHTTP_OPTION_REDIRECT_POLICY,
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
            ),
            (
                WINHTTP_OPTION_DISABLE_FEATURE,
                WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION,
            ),
        ] {
            if WinHttpSetOption(request.0, name, (&value as *const u32).cast(), 4) == 0 {
                return Err("update-network");
            }
        }
        let mut headers = if path.contains("/releases/tags/") || path.ends_with("/releases/latest")
        {
            String::from("Accept: application/vnd.github+json\r\n")
        } else {
            String::from("Accept: application/octet-stream\r\n")
        };
        if let Some(token) = authorization {
            if host != "api.github.com"
                || token.is_empty()
                || token.len() > 512
                || !token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err("update-auth-policy");
            }
            headers.push_str("Authorization: Bearer ");
            headers.push_str(token);
            headers.push_str("\r\n");
        }
        if let Some((from, to, _)) = range {
            headers.push_str(&format!("Range: bytes={from}-{to}\r\n"));
        }
        let mut utf16: Vec<u16> = headers.encode_utf16().collect();
        let sent = WinHttpSendRequest(
            request.0,
            utf16.as_ptr(),
            utf16.len() as u32,
            null(),
            0,
            0,
            0,
        );
        for c in &mut utf16 {
            std::ptr::write_volatile(c, 0);
        }
        for c in headers.as_bytes_mut() {
            std::ptr::write_volatile(c, 0);
        }
        if sent == 0 || WinHttpReceiveResponse(request.0, null_mut()) == 0 {
            return Err(network_error());
        }
        let mut code = 0u32;
        let mut len = 4u32;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut code as *mut u32).cast(),
            &mut len,
            null_mut(),
        ) == 0
        {
            return Err("update-response");
        }
        if std::env::var_os("FUZE_UPDATE_VERIFY_TRACE").is_some() {
            eprintln!(
                "update-check role={} status={} ranged={}",
                if host == "api.github.com" {
                    "api"
                } else {
                    "cdn"
                },
                code,
                range.is_some()
            );
        }
        if [301, 302, 303, 307, 308].contains(&code) {
            let mut value = [0u16; 8192];
            let mut length = (value.len() * 2) as u32;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_LOCATION,
                null(),
                value.as_mut_ptr().cast(),
                &mut length,
                null_mut(),
            ) == 0
                || length as usize > value.len() * 2
            {
                return Err("update-redirect");
            }
            return Ok(Some(
                String::from_utf16(&value[..length as usize / 2])
                    .map_err(|_| "update-redirect")?
                    .trim_end_matches('\0')
                    .to_owned(),
            ));
        }
        if code == 401 || code == 403 || code == 404 {
            return Err("update-access-or-release");
        }
        if code != if range.is_some() { 206 } else { 200 } {
            return Err("update-http-status");
        }
        if let Some((from, to, total)) = range {
            let mut value = [0u16; 128];
            let mut length = 256;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_CONTENT_RANGE,
                null(),
                value.as_mut_ptr().cast(),
                &mut length,
                null_mut(),
            ) == 0
            {
                return Err("update-range");
            }
            let actual = String::from_utf16_lossy(&value[..(length as usize / 2).min(value.len())])
                .trim_end_matches('\0')
                .to_owned();
            if actual != format!("bytes {from}-{to}/{total}") {
                return Err("update-range");
            }
        }
        let mut bytes = 0usize;
        let mut buffer = [0u8; 65536];
        loop {
            check()?;
            // Read the bytes already available rather than waiting to fill 64 KiB.
            // WinHTTP receive timeouts apply per network packet, not per ReadData call.
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return Err(network_error());
            }
            check()?;
            let mut read = 0;
            if WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                available.max(1).min(buffer.len() as u32),
                &mut read,
            ) == 0
            {
                return Err(network_error());
            }
            if read == 0 {
                break;
            }
            if bytes + read as usize > limit {
                return Err("update-size");
            }
            output
                .write_all(&buffer[..read as usize])
                .map_err(|_| "update-write")?;
            bytes += read as usize;
        }
        check()?;
        if range.is_some() && bytes != limit {
            return Err("update-range");
        }
        Ok(None)
    }
}
