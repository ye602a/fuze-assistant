//! Bounded startup-log fallback. Never log paths, bodies, ports or credentials.
use crate::{Credentials, Error, Result};
use std::{borrow::Cow, fs, io::Read, path::Path, time::SystemTime};

const MAX_BYTES: usize = 256 * 1024;
struct Secret(Vec<u8>);
impl Drop for Secret {
    fn drop(&mut self) {
        crate::wipe(&mut self.0);
    }
}
struct SecretText<'a>(Cow<'a, str>);
impl Drop for SecretText<'_> {
    fn drop(&mut self) {
        if let Cow::Owned(text) = &mut self.0 {
            // Filling with zero preserves UTF-8 and wipes the lossy decoding copy.
            unsafe {
                crate::wipe(text.as_bytes_mut());
            }
        }
    }
}
fn parse_bytes(pid: u32, bytes: &[u8]) -> Result<Credentials> {
    // Match Node's readFile(..., "utf8"): replace malformed sequences and continue.
    // A non-UTF8 installation path in the preamble must not hide later ASCII flags.
    let text = SecretText(String::from_utf8_lossy(bytes));
    parse(pid, &text.0)
}

fn matches_process(name: &str, pid: u32) -> bool {
    pid != 0 && name.ends_with(&format!("_{pid}_LeagueClientUx.log"))
}

fn parse(pid: u32, text: &str) -> Result<Credentials> {
    for line in text.lines() {
        if line.len() > 32768 {
            continue;
        }
        if line.contains("--app-port=") && line.contains("--remoting-auth-token=") {
            // The existing parser rejects duplicate flags and wipes temporary copies.
            return crate::command_line::parse(pid, line);
        }
        if let Some((_, auth)) = line.split_once("https://riot:") {
            let (secret, endpoint) = auth.split_once("@127.0.0.1:").ok_or(Error::AuthInput)?;
            let port = endpoint
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .ok_or(Error::AuthInput)?;
            let rest = &endpoint[port.len()..];
            if !rest.is_empty() && !rest.starts_with(['/', '"', '\'', ' ', '\r', '\t']) {
                return Err(Error::AuthInput);
            }
            return Credentials::new(pid, port.parse().map_err(|_| Error::AuthInput)?, secret);
        }
    }
    Err(Error::AuthInput)
}

pub(crate) fn credentials(
    root: &Path,
    pid: u32,
    started: Option<SystemTime>,
) -> Result<Credentials> {
    // Known client layouts only; no recursive disk scan. Caller supplies a live PID.
    // A log can be created before UX starts; require recent writes, not a newer
    // file creation time. Metadata denial must not disable directory fallback.
    let mut dirs = vec![
        root.to_owned(),
        root.join("LeagueClient"),
        root.join("Logs/LeagueClient Logs"),
        root.join("Logs"),
    ];
    if root
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("LeagueClient"))
    {
        if let Some(parent) = root.parent() {
            dirs.push(parent.to_owned());
            dirs.push(parent.join("Logs/LeagueClient Logs"));
            dirs.push(parent.join("Logs"));
        }
    }
    let mut candidates = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.take(4096).flatten() {
            if !matches_process(&entry.file_name().to_string_lossy(), pid) {
                continue;
            }
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file()
                || started.is_some_and(|start| !meta.modified().is_ok_and(|t| t >= start))
            {
                continue;
            }
            candidates.push((
                meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                entry.path(),
            ));
        }
    }
    candidates.sort_by(|a, b| b.cmp(a));
    for (_, path) in candidates.into_iter().take(3) {
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        let mut buffer = Secret(Vec::with_capacity(MAX_BYTES));
        if file
            .take(MAX_BYTES as u64)
            .read_to_end(&mut buffer.0)
            .is_ok()
        {
            if let Ok(credentials) = parse_bytes(pid, &buffer.0) {
                return Ok(credentials);
            }
        }
    }
    Err(Error::Discovery)
}

#[cfg(test)]
#[path = "client_log_review.rs"]
mod review;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lossy_decode_preserves_url_fallback_and_rejects_damage_inside_auth() {
        let c = parse_bytes(
            12,
            b"\xff\xfe\r\nhttps://riot:fixture@127.0.0.1:58124/\r\n\xe4\xb8",
        )
        .unwrap();
        assert_eq!(c.port, 58124);
        assert!(parse_bytes(12, b"--app-port=58124 --remoting-auth-token=fi\xffxture").is_err());
        assert!(parse_bytes(12, b"\xffhttps://riot:fixture@remote:58124/").is_err());
        assert!(matches!(
            SecretText(String::from_utf8_lossy(b"plain UTF8")).0,
            Cow::Borrowed(_)
        ));
    }
    #[test]
    fn log_forms_and_credential_boundaries() {
        for text in [
            "0000| OKAY| Command line arguments: --remoting-auth-token=fixture --app-port=58124",
            "Creating ux window with url https://riot:fixture@127.0.0.1:58124/bootstrap.html.",
            "Command line: \"LeagueClientUx.exe\" \"--app-port=58124\" \"--remoting-auth-token=fixture\"",
        ] {
            let c = parse(12, text).unwrap();
            assert!(c.belongs_to(12));
            assert_eq!(c.port, 58124);
            assert_eq!(format!("{c:?}"), "Credentials([redacted])");
        }
        for text in [
            "https://riot:fixture@remote:58124/",
            "https://riot:fixture@127.0.0.1:58124.evil/",
            "--app-port=58124 --app-port=58125 --remoting-auth-token=fixture",
            "https://riot:fixture@127.0.0.1:0/",
        ] {
            assert!(parse(12, text).is_err());
        }
        assert!(matches_process("date_12_LeagueClientUx.log", 12));
        assert!(!matches_process("date_112_LeagueClientUx.log", 12));
        assert!(!matches_process("date_12_LeagueClientUx.log.bak", 12));
    }
    #[test]
    fn non_utf8_preamble_does_not_hide_ascii_credentials() {
        let root = std::env::temp_dir().join(format!(
            "fuze-log-encoding-fixture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("date_12_LeagueClientUx.log");
        // Reproduce the observed byte layout, not any personal log content.
        let mut log = vec![b' '; 488];
        log.extend_from_slice(b"\xff\xfe non-UTF8 installation text\r\n");
        log.resize(1339, b' ');
        log.extend_from_slice(b"--remoting-auth-token=fixture --app-port=58124\r\n");
        fs::write(&path, log).unwrap();
        let result = credentials(&root, 12, None);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&root).unwrap();
        assert!(
            result.is_ok(),
            "an unrelated encoding error must not discard later credentials"
        );
        assert_eq!(result.unwrap().port, 58124);
    }
    #[test]
    fn fallback_uses_only_current_process_startup_logs() {
        let root = std::env::temp_dir().join(format!(
            "fuze-log-fixture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("Logs/LeagueClient Logs")).unwrap();
        let started = SystemTime::now() - std::time::Duration::from_secs(1);
        let path = root.join("Logs/LeagueClient Logs/date_12_LeagueClientUx.log");
        fs::write(
            &path,
            "Creating ux window https://riot:fixture@127.0.0.1:58124/bootstrap.html",
        )
        .unwrap();
        assert!(credentials(&root, 12, Some(started)).is_ok());
        assert!(credentials(&root, 13, Some(started)).is_err());
        // Pre-created startup files written by the current UX must remain usable.
        let later_start = SystemTime::now() + std::time::Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(later_start))
            .unwrap();
        assert!(credentials(&root, 12, Some(later_start)).is_ok());
        assert!(credentials(
            &root,
            12,
            Some(later_start + std::time::Duration::from_secs(1))
        )
        .is_err());
        let mut large = vec![b' '; MAX_BYTES];
        large.extend_from_slice(b"https://riot:fixture@127.0.0.1:58124/");
        fs::write(&path, large).unwrap();
        assert!(credentials(&root, 12, Some(started)).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(root.join("Logs/LeagueClient Logs")).unwrap();
        fs::remove_dir(root.join("Logs")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
