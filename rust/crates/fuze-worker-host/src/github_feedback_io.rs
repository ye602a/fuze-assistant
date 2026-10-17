//! Fixed GitHub Issues endpoint; no remote URLs, redirects, ambient credentials or raw errors.
use crate::{update_credential::Secret, Result};
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinHttp::*;
struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
fn handle(p: *mut c_void) -> Result<Handle> {
    if p.is_null() {
        Err("feedback-not-sent")
    } else {
        Ok(Handle(p))
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn request(
    method: &str,
    path: &str,
    body: &[u8],
    token: &Secret,
    cancel: &AtomicBool,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let cancelled =
        || cancel.load(Ordering::Acquire) || started.elapsed() > Duration::from_secs(15);
    if cancelled() {
        return Err("feedback-not-sent");
    }
    unsafe {
        let session = handle(WinHttpOpen(
            wide("Fuze-Rust-feedback").as_ptr(),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            null(),
            null(),
            0,
        ))?;
        if WinHttpSetTimeouts(session.0, 2000, 3000, 5000, 5000) == 0 {
            return Err("feedback-not-sent");
        }
        let connection = handle(WinHttpConnect(
            session.0,
            wide("api.github.com").as_ptr(),
            443,
            0,
        ))?;
        let req = handle(WinHttpOpenRequest(
            connection.0,
            wide(method).as_ptr(),
            wide(path).as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        for (key, value) in [
            (
                WINHTTP_OPTION_REDIRECT_POLICY,
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
            ),
            (
                WINHTTP_OPTION_DISABLE_FEATURE,
                WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION,
            ),
        ] {
            if WinHttpSetOption(req.0, key, (&value as *const u32).cast(), 4) == 0 {
                return Err("feedback-not-sent");
            }
        }
        if cancelled() {
            return Err("feedback-not-sent");
        }
        let mut header = format!("Accept: application/vnd.github+json\r\nContent-Type: application/json\r\nX-GitHub-Api-Version: 2022-11-28\r\nAuthorization: Bearer {}\r\n", token.text());
        let mut utf16: Vec<u16> = header.encode_utf16().collect();
        let sent = WinHttpSendRequest(
            req.0,
            utf16.as_ptr(),
            utf16.len() as u32,
            if body.is_empty() {
                null()
            } else {
                body.as_ptr().cast()
            },
            body.len() as u32,
            body.len() as u32,
            0,
        );
        let send_error = windows_sys::Win32::Foundation::GetLastError();
        for c in &mut utf16 {
            std::ptr::write_volatile(c, 0);
        }
        for c in header.as_bytes_mut() {
            std::ptr::write_volatile(c, 0);
        }
        // Any failure after send may have reached GitHub; never blindly repeat POST.
        if sent == 0
            && matches!(
                send_error,
                ERROR_WINHTTP_NAME_NOT_RESOLVED
                    | ERROR_WINHTTP_CANNOT_CONNECT
                    | ERROR_WINHTTP_SECURE_FAILURE
            )
        {
            return Err("feedback-not-sent");
        }
        if sent == 0 || WinHttpReceiveResponse(req.0, null_mut()) == 0 {
            return Err("feedback-network");
        }
        let mut code = 0u32;
        let mut size = 4u32;
        if WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut code as *mut u32).cast(),
            &mut size,
            null_mut(),
        ) == 0
        {
            return Err("feedback-network");
        }
        match code {
            200 | 201 => (),
            401 => return Err("feedback-auth"),
            403 | 404 => return Err("feedback-permission"),
            429 => return Err("feedback-rate-limit"),
            400 | 422 => return Err("feedback-rejected"),
            _ => return Err("feedback-network"),
        }
        let mut bytes = Vec::new();
        loop {
            if cancelled() {
                return Err("feedback-network");
            }
            let mut chunk = [0u8; 8192];
            let mut read = 0;
            if WinHttpReadData(
                req.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            ) == 0
            {
                return Err("feedback-network");
            }
            if read == 0 {
                break;
            }
            if bytes.len() + read as usize > 256 * 1024 {
                return Err("feedback-response");
            }
            bytes.extend_from_slice(&chunk[..read as usize]);
        }
        serde_json::from_slice(&bytes).map_err(|_| "feedback-response")
    }
}
pub(super) fn send(
    number: Option<u64>,
    body: &[u8],
    token: &Secret,
    cancel: &AtomicBool,
) -> Result<u64> {
    if body.len() > 60 * 1024 || number == Some(0) {
        return Err("feedback-invalid");
    }
    let base = "/repos/ye602a/fuze-assistant/issues";
    let path = number
        .map(|n| format!("{base}/{n}"))
        .unwrap_or_else(|| base.into());
    let value = request(
        if number.is_some() { "PATCH" } else { "POST" },
        &path,
        body,
        token,
        cancel,
    )?;
    let found = value["number"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or("feedback-response")?;
    if number.is_some_and(|n| n != found) {
        return Err("feedback-response");
    }
    Ok(found)
}
/// Read-only credential preflight. Repository access is not proof of Issues write permission.
pub(super) fn verify(token: &Secret, cancel: &AtomicBool) -> Result<()> {
    let value = request("GET", "/repos/ye602a/fuze-assistant", &[], token, cancel)?;
    if value["full_name"].as_str() != Some(super::REPOSITORY)
        || value["has_issues"].as_bool() != Some(true)
    {
        return Err("feedback-permission");
    }
    Ok(())
}
pub(super) fn lookup(
    id: &str,
    title: &str,
    token: &Secret,
    cancel: &AtomicBool,
) -> Result<Option<u64>> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("feedback-invalid");
    }
    let path = format!("/search/issues?q=repo:ye602a/fuze-assistant+in:title+{id}&per_page=5");
    let value = request("GET", &path, &[], token, cancel)?;
    lookup_result(&value, title)
}
fn lookup_result(value: &serde_json::Value, title: &str) -> Result<Option<u64>> {
    if value["incomplete_results"].as_bool() != Some(false) {
        return Err("feedback-response");
    }
    let items = value["items"].as_array().ok_or("feedback-response")?;
    let mut found = None;
    for item in items {
        if item["title"].as_str() == Some(title)
            && item["repository_url"].as_str()
                == Some("https://api.github.com/repos/ye602a/fuze-assistant")
            && item.get("pull_request").is_none()
        {
            if found.is_some() {
                return Err("feedback-response");
            }
            found = Some(
                item["number"]
                    .as_u64()
                    .filter(|n| *n > 0)
                    .ok_or("feedback-response")?,
            );
        }
    }
    Ok(found)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconciliation_requires_exact_title_repository_and_unique_issue() {
        let item = serde_json::json!({"title":"fixture","number":42,"repository_url":"https://api.github.com/repos/ye602a/fuze-assistant"});
        assert_eq!(
            lookup_result(
                &serde_json::json!({"incomplete_results":false,"items":[item]}),
                "fixture"
            )
            .unwrap(),
            Some(42)
        );
        assert_eq!(
            lookup_result(
                &serde_json::json!({"incomplete_results":false,"items":[item]}),
                "other"
            )
            .unwrap(),
            None
        );
        assert!(lookup_result(
            &serde_json::json!({"incomplete_results":false,"items":[item,item]}),
            "fixture"
        )
        .is_err());
    }
}
