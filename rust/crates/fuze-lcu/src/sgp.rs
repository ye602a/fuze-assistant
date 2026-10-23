//! Explicit history fallback only. Built-in v0.6.3 route allowlist, strict public TLS, no redirects.
use super::*;
fn route(platform: &str) -> Option<(&'static str, u16)> {
    Some(match platform {
        "HN1" => ("hn1-k8s-sgp.lol.qq.com", 21019),
        "HN10" => ("hn10-k8s-sgp.lol.qq.com", 21019),
        "TJ100" => ("tj100-sgp.lol.qq.com", 21019),
        "TJ101" => ("tj101-sgp.lol.qq.com", 21019),
        "NJ100" => ("nj100-sgp.lol.qq.com", 21019),
        "GZ100" => ("gz100-sgp.lol.qq.com", 21019),
        "CQ100" => ("cq100-sgp.lol.qq.com", 21019),
        "BGP2" => ("bgp2-k8s-sgp.lol.qq.com", 21019),
        "PBE" => ("pbe-sgp.lol.qq.com", 21019),
        "PREPBE" => ("prepbe-sgp.lol.qq.com", 21019),
        "TW2" | "SG2" | "PH2" | "VN2" | "OC1" | "TH2" => ("apse1-red.pp.sgp.pvp.net", 443),
        "PBE1" | "BR1" | "LA1" | "LA2" | "NA1" => ("usw2-red.pp.sgp.pvp.net", 443),
        "EUW1" | "RU" | "TR1" => ("euc1-red.pp.sgp.pvp.net", 443),
        "JP1" | "KR" => ("apne1-red.pp.sgp.pvp.net", 443),
        _ => return None,
    })
}
pub(super) fn fetch(
    owner: &crate::history::Owner,
    token: &serde_json::Value,
    start: u32,
    patch: Option<&str>,
) -> crate::history::Result<crate::contribution::Page> {
    let (host, port) = route(owner.platform()).ok_or("history-platform-unsupported")?;
    let bearer = token["accessToken"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 16384 && s.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or("history-auth-failed")?;
    let result = (|| -> Result<Vec<u8>> {
        unsafe {
            let session = Handle::new(WinHttpOpen(
                wide("Fuze-Rust-history").as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                null(),
                null(),
                0,
            ))?;
            ok(WinHttpSetTimeouts(session.0, 2000, 3000, 5000, 5000))?;
            let connection = Handle::new(WinHttpConnect(session.0, wide(host).as_ptr(), port, 0))?;
            let request = Handle::new(WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide(&owner.sgp_path(start)).as_ptr(),
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
            // Never copy the loopback certificate exceptions into this remote request.
            let mut header = Header("Authorization: Bearer ".encode_utf16().collect());
            header.0.extend(bearer.encode_utf16());
            header.0.extend("\r\n".encode_utf16());
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
            let mut size = 4;
            ok(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut code as *mut u32).cast(),
                &mut size,
                null_mut(),
            ))?;
            status(code)?;
            let started = Instant::now();
            let mut body = Vec::new();
            loop {
                if started.elapsed() > Duration::from_secs(5) {
                    wipe(&mut body);
                    return Err(Error::Transport);
                }
                let mut buffer = [0u8; 8192];
                let mut read = 0;
                if WinHttpReadData(request.0, buffer.as_mut_ptr().cast(), 8192, &mut read) == 0 {
                    wipe(&mut body);
                    return Err(Error::Transport);
                }
                if read == 0 {
                    return Ok(body);
                }
                if body.len() + read as usize > 4 * 1024 * 1024 {
                    wipe(&mut body);
                    return Err(Error::ResponseLimit);
                }
                body.extend_from_slice(&buffer[..read as usize]);
                wipe(&mut buffer);
            }
        }
    })();
    let mut body = result.map_err(|e| match e {
        Error::Unauthorized => "history-auth-failed",
        Error::ResponseLimit => "history-response-limit",
        _ => "history-sgp-unavailable",
    })?;
    let parsed = serde_json::from_slice(&body)
        .map_err(|_| "history-invalid-response")
        .and_then(|v| crate::contribution::page(owner, &v, patch));
    wipe(&mut body);
    parsed
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_never_guesses_region_or_accepts_an_origin() {
        for invalid in [
            "CN",
            "TENCENT",
            "HN2",
            "https://example.com",
            "hn1",
            "HN1@evil",
        ] {
            assert!(route(invalid).is_none());
        }
        assert_eq!(route("HN1"), Some(("hn1-k8s-sgp.lol.qq.com", 21019)));
    }
}
