//! One bounded, untrusted full-download prefix. Only an explicit updater run opens it.
//! No URLs, credentials, per-block hashing or background maintenance.
use crate::{managed_install, update_channel::Manifest, Result};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
};

const HEADER: u64 = 4096;
const BLOCK: u64 = 1024 * 1024;
const MAGIC: &[u8; 8] = b"FUZER001";
pub struct Cache {
    file: File,
    size: u64,
}
impl Cache {
    /// Caller holds the managed-install lock. Unknown/malformed files are never overwritten.
    pub fn open(root: &Path, manifest: &Manifest) -> Result<Self> {
        manifest.validate()?;
        managed_install::safe_dir(root)?;
        let path = root.join("download.resume");
        let options = || {
            let mut o = OpenOptions::new();
            o.read(true)
                .write(true)
                .share_mode(0)
                .custom_flags(0x00200000); // OPEN_REPARSE_POINT
            o
        };
        let (mut file, fresh) = match options().create_new(true).open(&path) {
            Ok(file) => (file, true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (
                options().open(&path).map_err(|_| "update-resume-open")?,
                false,
            ),
            Err(_) => return Err("update-resume-open"),
        };
        let metadata = file.metadata().map_err(|_| "update-resume-open")?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if !metadata.is_file()
            || metadata.file_attributes() & 0x400 != 0
            || unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0
            || info.nNumberOfLinks != 1
        {
            return Err("update-resume-path");
        }
        let identity = serde_json::to_vec(manifest).map_err(|_| "update-resume-header")?;
        if identity.len() > HEADER as usize - 12 {
            return Err("update-resume-header");
        }
        let mut header = [0u8; HEADER as usize];
        header[..8].copy_from_slice(MAGIC);
        header[8..12].copy_from_slice(&(identity.len() as u32).to_le_bytes());
        header[12..12 + identity.len()].copy_from_slice(&identity);
        if !fresh {
            let mut saved = [0u8; HEADER as usize];
            file.read_exact(&mut saved)
                .map_err(|_| "update-resume-header")?;
            let len = u32::from_le_bytes(saved[8..12].try_into().unwrap()) as usize;
            if &saved[..8] != MAGIC || len > HEADER as usize - 12 {
                return Err("update-resume-header");
            }
            let old: Manifest =
                serde_json::from_slice(&saved[12..12 + len]).map_err(|_| "update-resume-header")?;
            old.validate().map_err(|_| "update-resume-header")?;
            if metadata.len() < HEADER || metadata.len() > HEADER + old.size {
                return Err("update-resume-size");
            }
            if saved == header {
                return Ok(Self {
                    file,
                    size: manifest.size,
                });
            }
        }
        // Discard an old version's payload before changing its identity. A torn header
        // is ignored on the next run; it can never authorize extraction or installation.
        file.set_len(0).map_err(|_| "update-resume-write")?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| "update-resume-write")?;
        file.write_all(&header).map_err(|_| "update-resume-write")?;
        file.sync_all().map_err(|_| "update-resume-write")?;
        Ok(Self {
            file,
            size: manifest.size,
        })
    }
    pub fn clear(&mut self) -> Result<()> {
        self.file
            .set_len(HEADER)
            .map_err(|_| "update-resume-write")?;
        self.file.sync_all().map_err(|_| "update-resume-write")
    }
    /// Candidate byte count only, never an integrity or installation verdict.
    pub fn retained_bytes(&self) -> Result<u64> {
        self.file
            .metadata()
            .map_err(|_| "update-resume-open")?
            .len()
            .checked_sub(HEADER)
            .filter(|n| *n <= self.size)
            .ok_or("update-resume-size")
    }
    pub fn transfer(
        &mut self,
        output: &mut File,
        fetch: &dyn Fn(u64, &mut dyn Write) -> Result<()>,
        cancel: &dyn Fn() -> bool,
        reused: &dyn Fn(u64),
    ) -> Result<()> {
        if cancel() {
            return Err("update-cancelled");
        }
        let length = self.retained_bytes()?;
        // File length is NOT a verification signal. Drop an interrupted final block;
        // even a complete cached prefix remains untrusted until the caller's final hash.
        let mut prefix = if length == self.size {
            length
        } else {
            length / BLOCK * BLOCK
        };
        self.file
            .set_len(HEADER + prefix)
            .map_err(|_| "update-resume-write")?;
        self.file
            .seek(SeekFrom::Start(HEADER + prefix))
            .map_err(|_| "update-resume-write")?;
        reused(prefix);
        let mut result = fetch(prefix, &mut self.file);
        if prefix > 0 && result == Err("update-http-status") && !cancel() {
            self.clear()?;
            self.file
                .seek(SeekFrom::Start(HEADER))
                .map_err(|_| "update-resume-write")?;
            prefix = 0;
            reused(prefix);
            result = fetch(0, &mut self.file);
        }
        // One flush per attempt, not per network packet or block.
        self.file.sync_all().map_err(|_| "update-resume-write")?;
        result?;
        if self
            .file
            .metadata()
            .map_err(|_| "update-resume-open")?
            .len()
            != HEADER + self.size
        {
            return Err("update-size");
        }
        self.file
            .seek(SeekFrom::Start(HEADER))
            .map_err(|_| "update-resume-open")?;
        let mut remaining = self.size;
        let mut buffer = [0u8; 65536];
        while remaining > 0 {
            if cancel() {
                return Err("update-cancelled");
            }
            let count = remaining.min(buffer.len() as u64) as usize;
            self.file
                .read_exact(&mut buffer[..count])
                .map_err(|_| "update-resume-open")?;
            output
                .write_all(&buffer[..count])
                .map_err(|_| "update-write")?;
            remaining -= count as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{update_channel, update_io};
    use std::{
        cell::{Cell, RefCell},
        fs,
        path::PathBuf,
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "fuze-resume-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn output(&self, name: &str) -> File {
            OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(self.0.join(name))
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn manifest(bytes: &[u8]) -> Manifest {
        Manifest {
            schema: 1,
            channel: "rust-preview".into(),
            revision: "0123456789ab".into(),
            sequence: 1,
            archive: "Fuze-Rust-0123456789ab.fuz".into(),
            size: bytes.len() as u64,
            sha512: update_io::digest(bytes, true)
                .unwrap()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            blockmap_sha256: "00".repeat(32),
        }
    }
    #[test]
    fn update_resume_interruption_reopens_suffix_and_verifies_whole_package() {
        let f = Fixture::new();
        let bytes: Vec<u8> = (0..BLOCK as usize * 2 + 31)
            .map(|i| (i % 251) as u8)
            .collect();
        let m = manifest(&bytes);
        let mut cache = Cache::open(&f.0, &m).unwrap();
        let mut output = f.output("interrupted");
        assert_eq!(
            cache.transfer(
                &mut output,
                &|offset, out| {
                    assert_eq!(offset, 0);
                    out.write_all(&bytes[..BLOCK as usize + 113]).unwrap();
                    Err("update-cancelled-or-timeout")
                },
                &|| false,
                &|_| {}
            ),
            Err("update-cancelled-or-timeout")
        );
        assert_eq!(output.metadata().unwrap().len(), 0);
        drop(cache);
        let cache = RefCell::new(Cache::open(&f.0, &m).unwrap());
        let received = Cell::new(0);
        let result = update_channel::fetch_observed(
            &m,
            &f.0.join("verified"),
            None,
            &[],
            &|_, _| panic!("no delta"),
            &|out| {
                cache.borrow_mut().transfer(
                    out,
                    &|offset, output| {
                        assert_eq!(offset, BLOCK);
                        received.set(bytes.len() as u64 - offset);
                        output.write_all(&bytes[offset as usize..]).unwrap();
                        Ok(())
                    },
                    &|| false,
                    &|prefix| assert_eq!(prefix, BLOCK),
                )
            },
            &|| false,
            &|_| {},
        )
        .unwrap();
        assert_eq!(result.mode, "full");
        assert_eq!(received.get(), bytes.len() as u64 - BLOCK);
        assert_eq!(fs::read(f.0.join("verified")).unwrap(), bytes);
        cache.borrow_mut().clear().unwrap();
        assert_eq!(
            fs::metadata(f.0.join("download.resume")).unwrap().len(),
            HEADER
        );
    }
    #[test]
    fn update_resume_rejects_corruption_before_install_and_allows_fresh_retry() {
        let f = Fixture::new();
        let bytes = vec![7; BLOCK as usize + 31];
        let m = manifest(&bytes);
        let cache = RefCell::new(Cache::open(&f.0, &m).unwrap());
        cache
            .borrow_mut()
            .file
            .seek(SeekFrom::Start(HEADER))
            .unwrap();
        cache
            .borrow_mut()
            .file
            .write_all(&vec![9; BLOCK as usize])
            .unwrap();
        let result = update_channel::fetch_observed(
            &m,
            &f.0.join("bad"),
            None,
            &[],
            &|_, _| panic!(),
            &|out| {
                cache.borrow_mut().transfer(
                    out,
                    &|offset, output| {
                        output.write_all(&bytes[offset as usize..]).unwrap();
                        Ok(())
                    },
                    &|| false,
                    &|_| {},
                )
            },
            &|| false,
            &|_| {},
        );
        assert_eq!(result, Err("update-final-hash"));
        cache.borrow_mut().clear().unwrap();
        cache
            .borrow_mut()
            .transfer(
                &mut f.output("fresh"),
                &|offset, output| {
                    assert_eq!(offset, 0);
                    output.write_all(&bytes).unwrap();
                    Ok(())
                },
                &|| false,
                &|_| {},
            )
            .unwrap();
    }
    #[test]
    fn update_resume_range_rejection_restarts_once_but_auth_and_cancel_do_not() {
        let f = Fixture::new();
        let bytes = vec![3; BLOCK as usize + 11];
        let m = manifest(&bytes);
        let mut cache = Cache::open(&f.0, &m).unwrap();
        cache.file.seek(SeekFrom::Start(HEADER)).unwrap();
        cache.file.write_all(&bytes[..BLOCK as usize]).unwrap();
        let calls = Cell::new(0);
        let mut out = f.output("range");
        cache
            .transfer(
                &mut out,
                &|offset, output| {
                    calls.set(calls.get() + 1);
                    if offset > 0 {
                        return Err("update-http-status");
                    }
                    output.write_all(&bytes).unwrap();
                    Ok(())
                },
                &|| false,
                &|_| {},
            )
            .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(fs::read(f.0.join("range")).unwrap(), bytes);
        assert_eq!(
            cache.transfer(&mut out, &|_, _| panic!(), &|| true, &|_| {}),
            Err("update-cancelled")
        );
        calls.set(0);
        assert_eq!(
            cache.transfer(
                &mut out,
                &|_, _| {
                    calls.set(calls.get() + 1);
                    Err("update-access-or-release")
                },
                &|| false,
                &|_| {}
            ),
            Err("update-access-or-release")
        );
        assert_eq!(calls.get(), 1);
    }
    #[test]
    fn update_resume_identity_changes_reset_only_recognized_cache_and_reject_links() {
        let f = Fixture::new();
        let bytes = vec![1; BLOCK as usize + 1];
        let mut m = manifest(&bytes);
        let mut cache = Cache::open(&f.0, &m).unwrap();
        cache.file.seek(SeekFrom::Start(HEADER)).unwrap();
        cache.file.write_all(&bytes[..BLOCK as usize]).unwrap();
        drop(cache);
        m.sequence += 1;
        drop(Cache::open(&f.0, &m).unwrap());
        assert_eq!(
            fs::metadata(f.0.join("download.resume")).unwrap().len(),
            HEADER
        );
        fs::hard_link(f.0.join("download.resume"), f.0.join("alias")).unwrap();
        assert!(matches!(Cache::open(&f.0, &m), Err("update-resume-path")));
        fs::remove_file(f.0.join("alias")).unwrap();
        fs::write(f.0.join("download.resume"), b"unknown user content").unwrap();
        assert!(matches!(Cache::open(&f.0, &m), Err("update-resume-header")));
        assert_eq!(
            fs::read(f.0.join("download.resume")).unwrap(),
            b"unknown user content"
        );
    }
}
