//! Explicit private-channel credential UI. Public requests never open the credential store.
use crate::Result;
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{Foundation::*, Security::Credentials::*};
const TARGET: &str = "Fuze/Rust/GitHub/ye602a/fuze-assistant";
fn wide(s: &str) -> Vec<u16> {
    crate::i18n::text(s).encode_utf16().chain(Some(0)).collect()
}
pub struct Secret(Vec<u8>);
impl Secret {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let secret = Self(bytes);
        if secret.0.is_empty()
            || secret.0.len() > 512
            || !secret
                .0
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            return Err("credential-format");
        }
        Ok(secret)
    }
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        for b in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(b, 0);
            }
        }
    }
}
pub fn read() -> Result<Secret> {
    unsafe {
        let mut raw = null_mut();
        if CredReadW(wide(TARGET).as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) == 0 {
            return Err("credential-unavailable");
        }
        let result = if (*raw).CredentialBlobSize > 512 || (*raw).CredentialBlob.is_null() {
            Err("credential-format")
        } else {
            Secret::new(
                std::slice::from_raw_parts(
                    (*raw).CredentialBlob,
                    (*raw).CredentialBlobSize as usize,
                )
                .to_vec(),
            )
        };
        if !(*raw).CredentialBlob.is_null() {
            for i in 0..(*raw).CredentialBlobSize as usize {
                std::ptr::write_volatile((*raw).CredentialBlob.add(i), 0);
            }
        }
        CredFree(raw.cast());
        result
    }
}
pub fn remove() -> Result<()> {
    unsafe {
        if CredDeleteW(wide(TARGET).as_ptr(), CRED_TYPE_GENERIC, 0) == 0
            && GetLastError() != ERROR_NOT_FOUND
        {
            return Err("credential-delete");
        }
    }
    Ok(())
}
pub fn prompt() -> Result<()> {
    unsafe {
        let message = wide("仅私有频道需要：用户名可填 GitHub，密码栏填写仅此仓库 Contents 只读令牌。公开频道无需填写。");
        let caption = wide("符择 · 私有更新凭据");
        let info = CREDUI_INFOW {
            cbSize: size_of::<CREDUI_INFOW>() as u32,
            hwndParent: null_mut(),
            pszMessageText: message.as_ptr(),
            pszCaptionText: caption.as_ptr(),
            hbmBanner: null_mut(),
        };
        let mut user = [0u16; 128];
        let mut password = [0u16; 513];
        let mut save = 0;
        let code = CredUIPromptForCredentialsW(
            &info,
            wide(TARGET).as_ptr(),
            null(),
            0,
            user.as_mut_ptr(),
            user.len() as u32,
            password.as_mut_ptr(),
            password.len() as u32,
            &mut save,
            CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_DO_NOT_PERSIST
                | CREDUI_FLAGS_ALWAYS_SHOW_UI,
        );
        let length = password
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(password.len());
        let bytes = password[..length]
            .iter()
            .map(|c| if *c <= 127 { *c as u8 } else { 0 })
            .collect();
        for b in &mut password {
            std::ptr::write_volatile(b, 0);
        }
        for b in &mut user {
            std::ptr::write_volatile(b, 0);
        }
        let secret = Secret::new(bytes);
        if code == ERROR_CANCELLED {
            return Err("credential-cancelled");
        }
        if code != 0 {
            return Err("credential-dialog");
        }
        let secret = secret?;
        let mut target = wide(TARGET);
        let mut name = wide("GitHub");
        let mut credential: CREDENTIALW = zeroed();
        credential.Type = CRED_TYPE_GENERIC;
        credential.TargetName = target.as_mut_ptr();
        credential.UserName = name.as_mut_ptr();
        credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
        credential.CredentialBlobSize = secret.0.len() as u32;
        credential.CredentialBlob = secret.0.as_ptr() as *mut u8;
        if CredWriteW(&credential, 0) == 0 {
            return Err("credential-save");
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_header_injection_without_store_access() {
        assert!(Secret::new(b"fixture\r\nHost: other".to_vec()).is_err());
        assert!(Secret::new(vec![b'a'; 513]).is_err());
        assert!(Secret::new(b"fixture_only".to_vec()).is_ok());
    }
}
