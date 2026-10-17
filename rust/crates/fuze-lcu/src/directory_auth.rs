//! Directory fallback deliberately does not depend on OpenProcess/WMI access.
//! Candidates remain bound to a live Toolhelp observation and require API verification.
use crate::{Credentials, Error, Result};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Client,
    Ux,
}
impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Client => "LeagueClient.exe",
            Self::Ux => "LeagueClientUx.exe",
        }
    }
}
#[derive(Clone)]
pub(crate) struct Observation {
    pub pid: u32,
    pub parent: u32,
    pub kind: Kind,
    pub root: Option<PathBuf>,
    pub started: Option<SystemTime>,
}
pub(crate) struct Found {
    pub observation: Observation,
    pub credentials: Credentials,
    pub root: PathBuf,
}
struct Secret(Vec<u8>);
impl Drop for Secret {
    fn drop(&mut self) {
        crate::wipe(&mut self.0);
    }
}

pub(crate) fn discover(root: &Path, observed: &[Observation]) -> Result<Option<Found>> {
    let roots = crate::install::client_directories(root);
    if roots.is_empty() {
        return Err(Error::Discovery);
    }
    // If the image path is readable, never substitute a different installation.
    let matches: Vec<_> = observed
        .iter()
        .filter(|p| {
            p.pid != 0
                && p.root.as_ref().is_none_or(|p| {
                    roots
                        .iter()
                        .any(|r| match (r.canonicalize(), p.canonicalize()) {
                            (Ok(r), Ok(p)) => r == p,
                            _ => false,
                        })
                })
        })
        .collect();
    for kind in [Kind::Client, Kind::Ux] {
        if matches.iter().filter(|p| p.kind == kind).count() > 1 {
            return Err(Error::AmbiguousClient);
        }
    }
    for p in matches.iter().filter(|p| p.kind == Kind::Client) {
        let mut locks = vec![root.join("lockfile")];
        locks.extend(
            roots
                .iter()
                .filter(|r| r.as_path() != root)
                .map(|r| r.join("lockfile")),
        );
        if root
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("LeagueClient"))
        {
            if let Some(parent) = root.parent() {
                locks.push(parent.join("lockfile"));
            }
        }
        for lock in locks {
            let mut secret = Secret(Vec::with_capacity(1025));
            let Ok(file) = File::open(&lock) else {
                continue;
            };
            if file.take(1025).read_to_end(&mut secret.0).is_err() {
                continue;
            }
            let Ok(text) = std::str::from_utf8(&secret.0) else {
                continue;
            };
            if let Ok(credentials) = Credentials::from_lockfile(text, p.pid) {
                return Ok(Some(Found {
                    observation: (*p).clone(),
                    credentials,
                    root: p.root.clone().unwrap_or_else(|| roots[0].clone()),
                }));
            }
        }
    }
    for p in matches.iter().filter(|p| p.kind == Kind::Ux) {
        if let Ok(credentials) = crate::client_log::credentials(root, p.pid, p.started) {
            return Ok(Some(Found {
                observation: (*p).clone(),
                credentials,
                root: p.root.clone().unwrap_or_else(|| roots[0].clone()),
            }));
        }
    }
    if matches.is_empty() {
        Ok(None)
    } else {
        Err(Error::Discovery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "fuze-directory-fixture-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn write(&self, name: &str, value: &str) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, value).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only this test's newly-created temporary directory.
            assert!(self.0.starts_with(std::env::temp_dir()));
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn process(kind: Kind, pid: u32) -> Observation {
        Observation {
            pid,
            parent: 10,
            kind,
            root: None,
            started: None,
        }
    }
    fn fallback(root: &Path, rows: &[Observation]) -> Result<Option<Found>> {
        // The real orchestration must attempt directory IO even after automatic
        // discovery cannot open/query the process. No live process/API is involved.
        crate::discovery::with_directory_fallback(Err(Error::Discovery), Some(root), |r| {
            discover(r, rows)
        })
    }
    #[test]
    fn denied_process_metadata_still_reads_manual_lockfile_in_both_layouts() {
        for nested in [false, true] {
            let f = Fixture::new();
            let prefix = if nested { "LeagueClient/" } else { "" };
            f.write(&format!("{prefix}LeagueClient.exe"), "");
            f.write(
                &format!("{prefix}lockfile"),
                "LeagueClient:12:58124:fixture:https",
            );
            let result = fallback(&f.0, &[process(Kind::Client, 12)])
                .unwrap()
                .unwrap();
            assert!(result.credentials.belongs_to(12));
            assert_eq!(
                format!("{:?}", result.credentials),
                "Credentials([redacted])"
            );
            assert!(crate::install::is_client_directory(&f.0));
        }
    }
    #[test]
    fn denied_metadata_and_no_lockfile_still_reads_tencent_startup_logs() {
        for log in [
            "LeagueClient/date_24_LeagueClientUx.log",
            "Logs/LeagueClient Logs/date_24_LeagueClientUx.log",
            "Logs/date_24_LeagueClientUx.log",
            "date_24_LeagueClientUx.log",
        ] {
            let f = Fixture::new();
            f.write("LeagueClient/LeagueClientUx.exe", "");
            f.write(
                log,
                "Command line arguments: --app-port=58124 --remoting-auth-token=fixture",
            );
            let result = fallback(&f.0, &[process(Kind::Ux, 24)]).unwrap().unwrap();
            assert!(result.credentials.belongs_to(24));
        }
    }
    #[test]
    fn child_directory_can_use_parent_lockfile() {
        let f = Fixture::new();
        f.write("LeagueClient/LeagueClient.exe", "");
        f.write("lockfile", "LeagueClient:12:58124:fixture:https");
        assert!(fallback(&f.0, &[process(Kind::Client, 12)])
            .unwrap()
            .is_some());
        assert!(
            fallback(&f.0.join("LeagueClient"), &[process(Kind::Client, 12)])
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn wrong_pid_other_install_and_ambiguous_clients_are_rejected() {
        let f = Fixture::new();
        f.write("LeagueClient.exe", "");
        f.write("lockfile", "LeagueClient:12:58124:fixture:https");
        assert!(fallback(&f.0, &[process(Kind::Client, 13)]).is_err());
        assert!(fallback(&f.0, &[]).unwrap().is_none());
        assert!(matches!(
            fallback(
                &f.0,
                &[process(Kind::Client, 12), process(Kind::Client, 13)]
            ),
            Err(Error::AmbiguousClient)
        ));
        let other = Fixture::new();
        let mut p = process(Kind::Client, 12);
        p.root = Some(other.0.clone());
        assert!(fallback(&f.0, &[p]).unwrap().is_none());
        assert!(!crate::install::is_client_directory(&other.0));
    }
    #[test]
    fn dead_and_oversized_files_do_not_become_credentials() {
        let f = Fixture::new();
        f.write("LeagueClient.exe", "");
        f.write(
            "lockfile",
            &format!("LeagueClient:12:58124:{}:https", "x".repeat(2048)),
        );
        f.write(
            "date_25_LeagueClientUx.log",
            "--app-port=58124 --remoting-auth-token=fixture",
        );
        assert!(fallback(&f.0, &[process(Kind::Client, 12), process(Kind::Ux, 24)]).is_err());
    }
    #[test]
    fn successful_automatic_and_ambiguous_result_do_not_invoke_fallback() {
        use crate::discovery::with_directory_fallback as choose;
        assert_eq!(
            choose(Ok(Some(42)), Some(Path::new("fixture")), |_| panic!(
                "unexpected IO"
            ))
            .unwrap(),
            Some(42)
        );
        assert_eq!(
            choose::<u32>(
                Err(Error::AmbiguousClient),
                Some(Path::new("fixture")),
                |_| panic!("ambiguous bypass")
            ),
            Err(Error::AmbiguousClient)
        );
        assert_eq!(
            choose::<u32>(Err(Error::Discovery), None, |_| panic!("no directory")),
            Err(Error::Discovery)
        );
    }
}
