//! Remote downloads are separate from LCU: strict public TLS, no cookies or automatic redirects.
//! Release distribution follows the user's system proxy routing, without ambient authentication.
use crate::Result;
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{Networking::WinHttp::*, Security::Cryptography::*};
struct Handle(*mut c_void);
struct TransferTrace {
    enabled: bool,
    started: Instant,
    headers_ms: Option<u128>,
    api: bool,
    ranged: bool,
    bytes: usize,
}
impl Drop for TransferTrace {
    fn drop(&mut self) {
        if self.enabled {
            let elapsed = self.started.elapsed().as_millis();
            let headers = self.headers_ms.unwrap_or(elapsed);
            eprintln!(
                "update-transfer role={} ranged={} headers_ms={} body_ms={} bytes={}",
                if self.api { "api" } else { "cdn" },
                self.ranged,
                headers,
                elapsed.saturating_sub(headers),
                self.bytes
            );
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
fn access_type(host: &str) -> u32 {
    // Match the existing desktop updater's system routing only for fixed release hosts.
    // Other data requests retain their existing direct policy; LCU uses a separate client.
    match host {
        "api.github.com"
        | "release-assets.githubusercontent.com"
        | "objects.githubusercontent.com" => WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        _ => WINHTTP_ACCESS_TYPE_NO_PROXY,
    }
}
fn handle(p: *mut c_void) -> Result<Handle> {
    if p.is_null() {
        Err(network_error("open"))
    } else {
        Ok(Handle(p))
    }
}
fn network_error(stage: &str) -> &'static str {
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    if std::env::var_os("FUZE_UPDATE_VERIFY_TRACE").is_some() {
        // Fixed stage and numeric WinHTTP code only; never headers, URLs or response bodies.
        eprintln!("update-network stage={stage} code={code}");
    }
    if code == ERROR_WINHTTP_TIMEOUT {
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
    let mut trace = TransferTrace {
        enabled: std::env::var_os("FUZE_UPDATE_VERIFY_TRACE").is_some(),
        started,
        headers_ms: None,
        api: host == "api.github.com",
        ranged: range.is_some(),
        bytes: 0,
    };
    let budget = request_budget(limit, range.is_some());
    let check = || {
        let cancelled = cancel();
        if cancelled || started.elapsed() > budget {
            if std::env::var_os("FUZE_UPDATE_VERIFY_TRACE").is_some() {
                eprintln!(
                    "update-budget scope={} ms={}",
                    if cancelled { "caller" } else { "request" },
                    started.elapsed().as_millis()
                );
            }
            Err("update-cancelled-or-timeout")
        } else {
            Ok(())
        }
    };
    check()?;
    unsafe {
        let session = handle(WinHttpOpen(
            wide("Fuze-Rust-update").as_ptr(),
            access_type(host),
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
        if sent == 0 {
            return Err(network_error("send"));
        }
        if WinHttpReceiveResponse(request.0, null_mut()) == 0 {
            return Err(network_error("receive"));
        }
        trace.headers_ms = Some(started.elapsed().as_millis());
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
        if matches!(code, 401 | 403 | 404 | 407) {
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
        // Whole-response downloads use a fixed buffer directly. QueryDataAvailable
        // adds a call/internal copy and normally limits each read to at most 8 KiB.
        // Keep both wall budgets and WinHTTP receive timeouts; zero ReadData means EOF.
        read_body(
            limit,
            range.is_some(),
            output,
            &check,
            &mut |buffer| {
                let mut read = 0;
                if WinHttpReadData(
                    request.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut read,
                ) == 0
                {
                    Err(network_error("read"))
                } else {
                    Ok(read as usize)
                }
            },
            &mut |bytes| trace.bytes = bytes,
        )?;
        Ok(None)
    }
}

fn read_body(
    limit: usize,
    exact: bool,
    output: &mut dyn std::io::Write,
    check: &dyn Fn() -> Result<()>,
    read: &mut dyn FnMut(&mut [u8]) -> Result<usize>,
    accepted: &mut dyn FnMut(usize),
) -> Result<()> {
    let mut bytes = 0usize;
    let mut buffer = [0u8; 65536];
    loop {
        check()?;
        // At the limit, one extra byte distinguishes EOF from an oversized response.
        let capacity = (limit - bytes).saturating_add(1).min(buffer.len());
        let count = read(&mut buffer[..capacity])?;
        check()?;
        if count > capacity || count > limit - bytes {
            return Err("update-size");
        }
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|_| "update-write")?;
        bytes += count;
        accepted(bytes);
    }
    if exact && bytes != limit {
        return Err("update-range");
    }
    Ok(())
}

fn request_budget(limit: usize, ranged: bool) -> Duration {
    // A 1 MiB archive range is still a package transfer, not a metadata request.
    // Its caller shares a 600 s budget across metadata/delta/full. WinHTTP's existing
    // connect/send/receive timeouts remain active even while this wall budget is larger.
    Duration::from_secs(if ranged || limit > 8 * 1024 * 1024 {
        600
    } else {
        45
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_proxy_routing_is_confined_to_release_distribution() {
        for host in [
            "api.github.com",
            "release-assets.githubusercontent.com",
            "objects.githubusercontent.com",
        ] {
            assert_eq!(access_type(host), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY);
        }
        for host in [
            "data.dtodo.cn",
            "localhost",
            "127.0.0.1",
            "api.github.com.other",
            "other.example",
        ] {
            assert_eq!(access_type(host), WINHTTP_ACCESS_TYPE_NO_PROXY);
        }
    }
    #[test]
    fn fixed_buffer_preserves_short_read_eof_size_and_cancellation_boundaries() {
        use std::io::{Cursor, Read};
        let bytes = vec![3u8; 131089];
        let mut source = Cursor::new(&bytes);
        let mut output = Vec::new();
        read_body(
            bytes.len(),
            true,
            &mut output,
            &|| Ok(()),
            &mut |buf| {
                // Short reads are valid; only zero is EOF.
                let length = buf.len().min(12345);
                source
                    .read(&mut buf[..length])
                    .map_err(|_| "update-network")
            },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(output, bytes);
        let mut short = Cursor::new(b"abc");
        assert_eq!(
            read_body(
                4,
                true,
                &mut Vec::new(),
                &|| Ok(()),
                &mut |buf| Ok(short.read(buf).unwrap()),
                &mut |_| {}
            ),
            Err("update-range")
        );
        let mut long = Cursor::new(b"abcde");
        assert_eq!(
            read_body(
                4,
                false,
                &mut Vec::new(),
                &|| Ok(()),
                &mut |buf| Ok(long.read(buf).unwrap()),
                &mut |_| {}
            ),
            Err("update-size")
        );
        assert_eq!(
            read_body(
                4,
                true,
                &mut Vec::new(),
                &|| Err("update-cancelled"),
                &mut |_| panic!("cancelled before read"),
                &mut |_| {}
            ),
            Err("update-cancelled")
        );
        let cancelled = std::cell::Cell::new(false);
        let mut output = Vec::new();
        assert_eq!(
            read_body(
                4,
                true,
                &mut output,
                &|| if cancelled.get() {
                    Err("update-cancelled")
                } else {
                    Ok(())
                },
                &mut |buf| {
                    buf[0] = 1;
                    cancelled.set(true);
                    Ok(1)
                },
                &mut |_| {}
            ),
            Err("update-cancelled")
        );
        assert!(output.is_empty());
    }
    #[test]
    fn archive_ranges_keep_package_budget_without_relaxing_metadata() {
        assert_eq!(request_budget(1024 * 1024, true), Duration::from_secs(600));
        assert_eq!(request_budget(1024 * 1024, false), Duration::from_secs(45));
        assert_eq!(
            request_budget(48 * 1024 * 1024, false),
            Duration::from_secs(600)
        );
    }
}
