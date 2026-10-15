//! WMI metadata for one already verified LeagueClientUx PID; no shell or memory APIs.
use crate::{Credentials, Error, Result};
use windows::{
    core::{w, BSTR},
    Win32::System::{Com::*, Rpc::*, Variant::*, Wmi::*},
};
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
struct Args(Vec<String>);
impl Drop for Args {
    fn drop(&mut self) {
        for s in &mut self.0 {
            unsafe {
                crate::wipe(s.as_bytes_mut());
            }
        }
    }
}
fn parse(pid: u32, text: &str) -> Result<Credentials> {
    if text.len() > 32768 {
        return Err(Error::AuthInput);
    }
    let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut count = 0;
    let argv = unsafe {
        windows::Win32::UI::Shell::CommandLineToArgvW(
            windows::core::PCWSTR(wide.as_ptr()),
            &mut count,
        )
    };
    for c in &mut wide {
        unsafe {
            std::ptr::write_volatile(c, 0);
        }
    }
    if argv.is_null() {
        return Err(Error::AuthInput);
    }
    let mut args = Args(Vec::new());
    unsafe {
        for i in 0..count {
            let value = *argv.add(i as usize);
            let slice = value.as_wide();
            if count <= 256 {
                args.0.push(String::from_utf16_lossy(slice));
            }
            for j in 0..slice.len() {
                std::ptr::write_volatile(value.0.add(j), 0);
            }
        }
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            argv.cast(),
        )));
    }
    if count > 256 {
        return Err(Error::AuthInput);
    }
    Credentials::from_process_args(pid, "LeagueClientUx.exe", &args.0)
}
pub fn credentials(pid: u32) -> Result<Credentials> {
    let run = || -> windows::core::Result<Result<Credentials>> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let _apartment = Apartment;
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
            let empty = BSTR::new();
            let services = locator.ConnectServer(
                &BSTR::from("ROOT\\CIMV2"),
                &empty,
                &empty,
                &empty,
                0,
                &empty,
                None,
            )?;
            CoSetProxyBlanket(
                &services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                None,
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )?;
            let query = BSTR::from(format!("SELECT CommandLine FROM Win32_Process WHERE ProcessId = {pid} AND Name = 'LeagueClientUx.exe'"));
            let rows = services.ExecQuery(
                &BSTR::from("WQL"),
                &query,
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )?;
            let mut result = [None];
            let mut count = 0;
            rows.Next(1000, &mut result, &mut count).ok()?;
            if count != 1 {
                return Ok(Err(Error::Discovery));
            }
            let row = result[0].as_ref().unwrap();
            let mut value = VARIANT::default();
            row.Get(w!("CommandLine"), 0, &mut value, None, None)?;
            let parsed = if value.Anonymous.Anonymous.vt == VT_BSTR {
                let bstr = &value.Anonymous.Anonymous.Anonymous.bstrVal;
                let mut text = bstr.to_string();
                let parsed = parse(pid, &text);
                crate::wipe(text.as_bytes_mut());
                for i in 0..bstr.len() {
                    std::ptr::write_volatile(bstr.as_ptr().add(i) as *mut u16, 0);
                }
                parsed
            } else {
                Err(Error::AuthInput)
            };
            VariantClear(&mut value)?;
            Ok(parsed)
        }
    };
    run().map_err(|_| Error::Discovery)?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_metadata_is_reduced_and_ambiguous_flags_rejected() {
        let c = parse(
            7,
            r#""C:\Games --app-port=9\LeagueClientUx.exe" --app-port=1234 --remoting-auth-token=fixture"#,
        )
        .unwrap();
        assert!(c.belongs_to(7));
        assert_eq!(format!("{c:?}"), "Credentials([redacted])");
        assert!(parse(7, "--app-port=1 --app-port=2 --remoting-auth-token=fixture").is_err());
        assert!(parse(7, "--app-port=0 --remoting-auth-token=fixture").is_err());
    }
}
