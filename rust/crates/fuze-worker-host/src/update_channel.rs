//! Fixed repository, explicit channel, bounded redirect session and one full-download fallback.
use crate::{
    private_update::{self, Target},
    update_credential::Secret,
    update_io, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
pub const RELEASE: &str =
    "https://api.github.com/repos/ye602a/fuze-assistant/releases/tags/rust-preview";
pub struct Session {
    secret: Option<Secret>,
}
struct ArchiveRoute<'a> {
    original: &'a str,
    resolved: std::sync::OnceLock<String>,
}
impl<'a> ArchiveRoute<'a> {
    fn new(original: &'a str) -> Self {
        Self {
            original,
            resolved: std::sync::OnceLock::new(),
        }
    }
    fn fetch<T>(&self, request: impl FnOnce(&str) -> Result<(String, T)>) -> Result<T> {
        let (target, value) = request(self.resolved.get().map_or(self.original, String::as_str))?;
        // The serial probe has already validated the response before publishing its route.
        self.resolved.get_or_init(|| target);
        Ok(value)
    }
}
struct Received<'a> {
    out: &'a mut dyn Write,
    progress: &'a (dyn Fn(u64) + Sync),
}
impl Write for Received<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = self.out.write(bytes)?;
        (self.progress)(count as u64);
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}
impl Session {
    /// Explicit developer verification uses an anonymous pipe, never argv or a saved credential.
    pub fn with_secret(secret: Secret) -> Self {
        Self {
            secret: Some(secret),
        }
    }
    pub fn public() -> Self {
        Self { secret: None }
    }
    pub(crate) fn uses_credential(&self) -> bool {
        self.secret.is_some()
    }
    pub fn private() -> Result<Self> {
        Ok(Self {
            secret: Some(crate::update_credential::read()?),
        })
    }
    pub fn download(
        &self,
        url: &str,
        limit: usize,
        range: Option<(u64, u64, u64)>,
        out: &mut dyn Write,
        cancel: &dyn Fn() -> bool,
    ) -> Result<()> {
        self.download_resolved(url, limit, range, out, cancel)
            .map(|_| ())
    }
    fn download_resolved(
        &self,
        url: &str,
        limit: usize,
        range: Option<(u64, u64, u64)>,
        out: &mut dyn Write,
        cancel: &dyn Fn() -> bool,
    ) -> Result<String> {
        self.download_resolved_with(url, out, &|host, path, auth, output| {
            update_io::transfer(host, path, limit, auth, range, cancel, output)
        })
    }
    #[cfg(test)]
    fn download_with(
        &self,
        url: &str,
        out: &mut dyn Write,
        transfer: &dyn Fn(&str, &str, Option<&str>, &mut dyn Write) -> Result<Option<String>>,
    ) -> Result<()> {
        self.download_resolved_with(url, out, transfer).map(|_| ())
    }
    fn download_resolved_with(
        &self,
        url: &str,
        out: &mut dyn Write,
        transfer: &dyn Fn(&str, &str, Option<&str>, &mut dyn Write) -> Result<Option<String>>,
    ) -> Result<String> {
        let mut current = url.to_owned();
        let mut cdn = false;
        for _ in 0..4 {
            let (host, path, auth) = match private_update::target(&current)? {
                Target::Api(p) if !cdn => {
                    ("api.github.com", p, self.secret.as_ref().map(Secret::text))
                }
                Target::Cdn { host, path } => {
                    cdn = true;
                    (host, path, None)
                }
                _ => return Err("update-redirect-policy"),
            };
            match transfer(host, path, auth, out)? {
                None => return Ok(current),
                Some(next) => {
                    if !matches!(private_update::target(&next)?, Target::Cdn { .. }) {
                        return Err("update-redirect-policy");
                    }
                    current = next;
                }
            }
        }
        Err("update-redirect-limit")
    }
    pub fn bytes(&self, url: &str, limit: usize, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>> {
        let mut b = Vec::new();
        self.download(url, limit, None, &mut b, cancel)?;
        Ok(b)
    }
    /// Full archive transfer with bounded range requests. All bytes are downloaded; no baseline
    /// is trusted. This avoids a single slow response holding an update worker for minutes.
    pub fn archive(
        &self,
        url: &str,
        size: u64,
        out: &mut dyn Write,
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<()> {
        self.archive_observed(url, size, out, cancel, &|_| {})
    }
    pub fn archive_observed(
        &self,
        url: &str,
        size: u64,
        out: &mut dyn Write,
        cancel: &(dyn Fn() -> bool + Sync),
        progress: &(dyn Fn(u64) + Sync),
    ) -> Result<()> {
        self.archive_from_observed(url, size, 0, out, cancel, progress)
    }
    /// Append only the requested suffix; the caller must still verify the whole package.
    pub fn archive_from_observed(
        &self,
        url: &str,
        size: u64,
        offset: u64,
        out: &mut dyn Write,
        cancel: &(dyn Fn() -> bool + Sync),
        progress: &(dyn Fn(u64) + Sync),
    ) -> Result<()> {
        if size == 0 || size > 512 * 1024 * 1024 || offset > size {
            return Err("update-size");
        }
        if cancel() {
            return Err("update-cancelled");
        }
        if offset == size {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let cancelled = || cancel() || started.elapsed() > std::time::Duration::from_secs(600);
        // Filled only after the serial probe succeeds. Never persisted or shared with
        // another archive; signed CDN addresses expire and must not become a disk cache.
        let resolved = ArchiveRoute::new(url);
        let fetch = |offset, length| {
            resolved.fetch(|target| {
                let mut bytes = Vec::with_capacity(length as usize);
                let final_url = self.download_resolved(
                    target,
                    length as usize,
                    Some((offset, offset + length - 1, size)),
                    &mut Received {
                        out: &mut bytes,
                        progress,
                    },
                    &cancelled,
                )?;
                Ok((final_url, bytes))
            })
        };
        let first = (size - offset).min(1024 * 1024);
        match fetch(offset, first) {
            Ok(bytes) => out.write_all(&bytes).map_err(|_| "update-write")?,
            // Probe once before concurrency; no partial 200 body has been written.
            Err("update-http-status") if offset == 0 => {
                return self.download(
                    url,
                    size as usize,
                    None,
                    &mut Received { out, progress },
                    &cancelled,
                )
            }
            Err(error) => return Err(error),
        }
        transfer_chunks(offset + first, size, out, &fetch, &cancelled)
    }
    pub fn check(&self, cancel: &dyn Fn() -> bool) -> Result<Release> {
        let raw: serde_json::Value =
            serde_json::from_slice(&self.bytes(RELEASE, 2 * 1024 * 1024, cancel)?)
                .map_err(|_| "update-release-format")?;
        if raw["tag_name"] != "rust-preview" || raw["draft"] != false || raw["prerelease"] != true {
            return Err("update-release-channel");
        }
        let assets = raw["assets"]
            .as_array()
            .filter(|a| a.len() <= 100)
            .ok_or("update-release-assets")?;
        let asset = |name: &str| -> Result<(String, u64)> {
            let list: Vec<_> = assets.iter().filter(|a| a["name"] == name).collect();
            if list.len() != 1 {
                return Err("update-release-assets");
            }
            let id = list[0]["id"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or("update-release-assets")?;
            let size = list[0]["size"].as_u64().ok_or("update-release-assets")?;
            Ok((
                format!("https://api.github.com/repos/ye602a/fuze-assistant/releases/assets/{id}"),
                size,
            ))
        };
        let (url, size) = asset("rust-update.json")?;
        if size > 16384 {
            return Err("update-manifest-size");
        }
        let manifest: Manifest = serde_json::from_slice(&self.bytes(&url, 16384, cancel)?)
            .map_err(|_| "update-manifest-format")?;
        manifest.validate()?;
        let (archive, bytes) = asset(&manifest.archive)?;
        let (map, mapbytes) = asset(&(manifest.archive.clone() + ".blockmap"))?;
        if bytes != manifest.size || mapbytes > 8 * 1024 * 1024 {
            return Err("update-asset-size");
        }
        Ok(Release {
            manifest,
            archive,
            map,
        })
    }
}
/// Ordered writes, at most four 1 MiB buffers and four scoped IO workers; never a game-loop task.
fn transfer_chunks(
    mut offset: u64,
    total: u64,
    out: &mut dyn Write,
    fetch: &(dyn Fn(u64, u64) -> Result<Vec<u8>> + Sync),
    cancel: &dyn Fn() -> bool,
) -> Result<()> {
    std::thread::scope(|scope| {
        let mut jobs = std::collections::VecDeque::new();
        let mut error = None;
        loop {
            if cancel() {
                error = Some("update-cancelled-or-timeout");
            }
            // Refill after each ordered write, without waiting for the slowest member
            // of a four-request batch. Pending/completed buffers remain bounded by four.
            while error.is_none() && jobs.len() < 4 && offset < total {
                let start = offset;
                let length = (total - offset).min(1024 * 1024);
                jobs.push_back((length, scope.spawn(move || fetch(start, length))));
                offset += length;
            }
            let Some((length, job)) = jobs.pop_front() else {
                break;
            };
            let block = job
                .join()
                .unwrap_or(Err("update-download-worker"))
                .and_then(|bytes| {
                    if bytes.len() as u64 == length {
                        Ok(bytes)
                    } else {
                        Err("update-range")
                    }
                });
            if cancel() {
                error = Some("update-cancelled-or-timeout");
            }
            match block {
                Ok(bytes) if error.is_none() => {
                    if out.write_all(&bytes).is_err() {
                        error = Some("update-write");
                    }
                }
                Err(e) => {
                    // Drain all in-flight workers after failure. An access-policy error
                    // must not be hidden by a transient error and cause a full retry.
                    if error.is_none_or(|prior| can_retry_full(prior) && !can_retry_full(e)) {
                        error = Some(e);
                    }
                }
                _ => {}
            }
        }
        error.map_or(Ok(()), Err)
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub channel: String,
    pub revision: String,
    pub sequence: u64,
    pub archive: String,
    pub size: u64,
    pub sha512: String,
    pub blockmap_sha256: String,
}
pub fn revision(v: &str) -> bool {
    v.len() == 12
        && v.bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
pub fn decode_hex(v: &str, count: usize) -> Result<Vec<u8>> {
    if v.len() != count * 2 || !v.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("update-digest-format");
    }
    (0..count)
        .map(|i| u8::from_str_radix(&v[i * 2..i * 2 + 2], 16).map_err(|_| "update-digest-format"))
        .collect()
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1
            || self.channel != "rust-preview"
            || !revision(&self.revision)
            || self.sequence == 0
            || self.archive != format!("Fuze-Rust-{}.fuz", self.revision)
            || self.size == 0
            || self.size > 512 * 1024 * 1024
        {
            return Err("update-manifest-policy");
        }
        decode_hex(&self.sha512, 64)?;
        decode_hex(&self.blockmap_sha256, 32)?;
        Ok(())
    }
}
pub struct Release {
    pub manifest: Manifest,
    pub archive: String,
    pub map: String,
}
#[derive(Serialize, Debug, PartialEq)]
pub struct Transfer {
    pub mode: &'static str,
    pub downloaded: u64,
    pub fallback: bool,
}
type RangeFetch<'a> = dyn Fn(u64, u64) -> Result<Vec<u8>> + 'a;
type FullFetch<'a> = dyn Fn(&mut File) -> Result<()> + 'a;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransferEvent {
    Delta { size: u64, reuse: u64 },
    Full { size: u64, fallback: bool },
    Verify,
}
// Local output failures, cancellation, access and redirect-policy errors must not
// silently start another download. Only recoverable delta/transport faults retry.
fn can_retry_full(error: &str) -> bool {
    matches!(
        error,
        "update-map-hash"
            | "delta-map-gzip"
            | "delta-map-limit"
            | "delta-map-format"
            | "delta-baseline"
            | "delta-range"
            | "delta-final-hash"
            | "delta-size"
            | "update-network"
            | "update-http-status"
            | "update-range"
    )
}
/// Caller owns a new staging file; cancellation is terminal and never triggers a full retry.
pub fn fetch_with(
    manifest: &Manifest,
    destination: &Path,
    baseline: Option<(&Path, &[u8])>,
    new_map: &[u8],
    range: &RangeFetch<'_>,
    full: &FullFetch<'_>,
    cancel: &dyn Fn() -> bool,
) -> Result<Transfer> {
    fetch_observed(
        manifest,
        destination,
        baseline,
        new_map,
        range,
        full,
        cancel,
        &|_| {},
    )
}
pub fn fetch_observed(
    manifest: &Manifest,
    destination: &Path,
    baseline: Option<(&Path, &[u8])>,
    new_map: &[u8],
    range: &RangeFetch<'_>,
    full: &FullFetch<'_>,
    cancel: &dyn Fn() -> bool,
    notify: &dyn Fn(TransferEvent),
) -> Result<Transfer> {
    manifest.validate()?;
    if cancel() {
        return Err("update-cancelled");
    }
    let expected = decode_hex(&manifest.sha512, 64)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(destination)
        .map_err(|_| "update-stage")?;
    let mut fallback = false;
    if let Some((old, map)) = baseline {
        let attempt = (|| {
            if update_io::digest(new_map, false)? != decode_hex(&manifest.blockmap_sha256, 32)? {
                return Err("update-map-hash");
            }
            let mut old = File::open(old).map_err(|_| "delta-baseline")?;
            let plan = private_update::plan(
                map,
                old.metadata().map_err(|_| "delta-baseline")?.len(),
                new_map,
                manifest.size,
            )?;
            let reuse = plan
                .iter()
                .map(|op| match op {
                    private_update::Operation::Copy { length, .. } => *length,
                    _ => 0,
                })
                .sum();
            notify(TransferEvent::Delta {
                size: manifest.size,
                reuse,
            });
            private_update::reconstruct_observed(
                &mut old,
                &mut output,
                &plan,
                manifest.size,
                &expected,
                range,
                cancel,
                &|| notify(TransferEvent::Verify),
            )
        })();
        match attempt {
            Ok(downloaded) => {
                output.sync_all().map_err(|_| "update-stage")?;
                return Ok(Transfer {
                    mode: "delta",
                    downloaded,
                    fallback: false,
                });
            }
            Err(error) => {
                if cancel() {
                    return Err("update-cancelled");
                }
                if !can_retry_full(error) {
                    return Err(error);
                }
                fallback = true;
            }
        }
        output.set_len(0).map_err(|_| "update-stage")?;
        output
            .seek(SeekFrom::Start(0))
            .map_err(|_| "update-stage")?;
    }
    if cancel() {
        return Err("update-cancelled");
    }
    notify(TransferEvent::Full {
        size: manifest.size,
        fallback,
    });
    full(&mut output)?;
    if cancel() {
        return Err("update-cancelled");
    }
    if output.metadata().map_err(|_| "update-stage")?.len() != manifest.size {
        return Err("update-size");
    }
    notify(TransferEvent::Verify);
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| "update-stage")?;
    if private_update::hash_reader(&mut output, cancel)? != expected {
        return Err("update-final-hash");
    }
    output.sync_all().map_err(|_| "update-stage")?;
    Ok(Transfer {
        mode: "full",
        downloaded: manifest.size,
        fallback,
    })
}
pub fn bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let mut b = Vec::new();
    File::open(path)
        .map_err(|_| "update-read")?
        .take(max + 1)
        .read_to_end(&mut b)
        .map_err(|_| "update-read")?;
    if b.len() as u64 > max {
        return Err("update-size");
    }
    Ok(b)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn archive_reuses_validated_route_without_reauthorizing_cdn_or_retrying_denial() {
        let session = Session::with_secret(Secret::new(b"fixture".to_vec()).unwrap());
        let route = ArchiveRoute::new(RELEASE);
        let api = std::sync::atomic::AtomicUsize::new(0);
        let cdn = std::sync::atomic::AtomicUsize::new(0);
        let fetch = |_, length| {
            route.fetch(|url| {
                let mut bytes = Vec::new();
                let target =
                    session.download_resolved_with(url, &mut bytes, &|host, _, auth, out| {
                        if host == "api.github.com" {
                            assert!(auth.is_some());
                            api.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(
                                "https://release-assets.githubusercontent.com/fixture".into(),
                            ))
                        } else {
                            assert!(auth.is_none());
                            cdn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            out.write_all(&vec![7; length as usize]).unwrap();
                            Ok(None)
                        }
                    })?;
                Ok((target, bytes))
            })
        };
        assert_eq!(fetch(0, 16).unwrap(), vec![7; 16]); // Serial probe publishes the route.
        let mut out = Vec::new();
        transfer_chunks(16, 4 * 1024 * 1024 + 17, &mut out, &fetch, &|| false).unwrap();
        assert_eq!(api.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(cdn.load(std::sync::atomic::Ordering::Relaxed), 6);
        assert!(out.iter().all(|b| *b == 7));
        let denied: Result<()> = route.fetch(|url| {
            session
                .download_resolved_with(url, &mut Vec::new(), &|host, _, auth, _| {
                    assert_ne!(host, "api.github.com");
                    assert!(auth.is_none());
                    Err("update-access-or-release")
                })
                .map(|target| (target, ()))
        });
        assert_eq!(denied, Err("update-access-or-release"));
        assert!(ArchiveRoute::new(RELEASE).resolved.get().is_none());
        let failed = ArchiveRoute::new(RELEASE);
        assert_eq!(
            failed.fetch::<()>(|_| Err("update-range")),
            Err("update-range")
        );
        assert!(failed.resolved.get().is_none());
    }
    #[test]
    fn redirects_strip_authorization_and_never_return_to_api() {
        let session = Session::with_secret(Secret::new(b"fixture".to_vec()).unwrap());
        let calls = Cell::new(0);
        let mut output = Vec::new();
        session
            .download_with(RELEASE, &mut output, &|host, _, auth, out| {
                calls.set(calls.get() + 1);
                if host == "api.github.com" {
                    assert!(auth.is_some());
                    Ok(Some(
                        "https://release-assets.githubusercontent.com/fixture".into(),
                    ))
                } else {
                    assert!(auth.is_none());
                    out.write_all(b"fixture").unwrap();
                    Ok(None)
                }
            })
            .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(output, b"fixture");
        assert_eq!(
            session.download_with(
                "https://release-assets.githubusercontent.com/fixture",
                &mut Vec::new(),
                &|_, _, auth, _| {
                    assert!(auth.is_none());
                    Ok(Some(RELEASE.into()))
                }
            ),
            Err("update-redirect-policy")
        );
        let calls = Cell::new(0);
        assert_eq!(
            Session::public().download_with(RELEASE, &mut Vec::new(), &|_, _, auth, _| {
                assert!(auth.is_none());
                calls.set(calls.get() + 1);
                Ok(Some(
                    "https://release-assets.githubusercontent.com/loop".into(),
                ))
            }),
            Err("update-redirect-limit")
        );
        assert_eq!(calls.get(), 4);
    }
    fn blockmap(last: &str) -> Vec<u8> {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        serde_json::to_writer(
            &mut gzip,
            &serde_json::json!({"version":"2","files":[{
            "name":"file","offset":0,"sizes":[4,4],"checksums":["same",last]}]}),
        )
        .unwrap();
        gzip.finish().unwrap()
    }
    #[test]
    fn delta_progress_and_terminal_errors_preserve_retry_boundary() {
        let dir =
            std::env::temp_dir().join(format!("fuze-transfer-boundary-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let old = dir.join("old");
        std::fs::write(&old, b"AAAABBBB").unwrap();
        let old_map = blockmap("old");
        let map = blockmap("new");
        let payload = b"AAAACCCC";
        let manifest = Manifest {
            schema: 1,
            channel: "rust-preview".into(),
            revision: "0123456789ab".into(),
            sequence: 2,
            archive: "Fuze-Rust-0123456789ab.fuz".into(),
            size: 8,
            sha512: update_io::hex(&update_io::digest(payload, true).unwrap()),
            blockmap_sha256: update_io::hex(&update_io::digest(&map, false).unwrap()),
        };
        let events = std::cell::RefCell::new(Vec::new());
        let path = dir.join("delta");
        let result = fetch_observed(
            &manifest,
            &path,
            Some((&old, &old_map)),
            &map,
            &|offset, length| {
                assert_eq!((offset, length), (4, 4));
                Ok(b"CCCC".to_vec())
            },
            &|_| panic!("valid delta must not download full"),
            &|| false,
            &|e| events.borrow_mut().push(e),
        )
        .unwrap();
        assert_eq!(
            result,
            Transfer {
                mode: "delta",
                downloaded: 4,
                fallback: false
            }
        );
        assert_eq!(
            *events.borrow(),
            vec![
                TransferEvent::Delta { size: 8, reuse: 4 },
                TransferEvent::Verify
            ]
        );
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        std::fs::remove_file(path).unwrap();
        for error in [
            "update-cancelled-or-timeout",
            "delta-cancelled",
            "update-access-or-release",
            "update-redirect-policy",
            "private-url",
            "update-write",
            "delta-write",
            "delta-hash",
        ] {
            let path = dir.join("terminal");
            assert_eq!(
                fetch_with(
                    &manifest,
                    &path,
                    Some((&old, &old_map)),
                    &map,
                    &|_, _| Err(error),
                    &|_| panic!("terminal error must not retry"),
                    &|| false
                ),
                Err(error)
            );
            std::fs::remove_file(path).unwrap();
        }
        // The copied baseline is corrupt: reuse must not bypass the final package digest.
        std::fs::write(&old, b"XXXXBBBB").unwrap();
        let full = Cell::new(0);
        let path = dir.join("fallback");
        events.borrow_mut().clear();
        let result = fetch_observed(
            &manifest,
            &path,
            Some((&old, &old_map)),
            &map,
            &|_, _| Ok(b"CCCC".to_vec()),
            &|out| {
                full.set(full.get() + 1);
                out.write_all(payload).unwrap();
                Ok(())
            },
            &|| false,
            &|e| events.borrow_mut().push(e),
        )
        .unwrap();
        assert_eq!(full.get(), 1);
        assert!(result.fallback);
        assert_eq!(
            *events.borrow(),
            vec![
                TransferEvent::Delta { size: 8, reuse: 4 },
                TransferEvent::Verify,
                TransferEvent::Full {
                    size: 8,
                    fallback: true
                },
                TransferEvent::Verify
            ]
        );
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(old).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn bounded_parallel_chunks_preserve_order_and_reject_short_ranges() {
        let total = 4 * 1024 * 1024 + 17;
        let active = std::sync::atomic::AtomicUsize::new(0);
        let peak = std::sync::atomic::AtomicUsize::new(0);
        let mut out = Vec::new();
        transfer_chunks(
            0,
            total,
            &mut out,
            &|offset, length| {
                let count = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                peak.fetch_max(count, std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(5));
                let result = vec![(offset / (1024 * 1024)) as u8; length as usize];
                active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                Ok(result)
            },
            &|| false,
        )
        .unwrap();
        assert_eq!(out.len() as u64, total);
        for (i, block) in out.chunks(1024 * 1024).enumerate() {
            assert!(block.iter().all(|b| *b == i as u8));
        }
        assert!(peak.load(std::sync::atomic::Ordering::SeqCst) <= 4);
        assert_eq!(
            transfer_chunks(0, 16, &mut Vec::new(), &|_, _| Ok(vec![]), &|| false),
            Err("update-range")
        );
        assert_eq!(
            transfer_chunks(0, 16, &mut Vec::new(), &|_, _| panic!(), &|| true),
            Err("update-cancelled-or-timeout")
        );
    }
    #[test]
    fn ordered_window_refills_before_later_blocks_and_drains_terminal_errors() {
        use std::sync::{mpsc, Mutex};
        let (started, ready) = mpsc::channel();
        let ready = Mutex::new(ready);
        let mut out = Vec::new();
        transfer_chunks(
            0,
            5 * 1024 * 1024,
            &mut out,
            &|offset, length| {
                let index = offset / (1024 * 1024);
                if index == 1 {
                    ready
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .map_err(|_| "validation-window-stalled")?;
                }
                if index == 4 {
                    started.send(()).unwrap();
                }
                Ok(vec![index as u8; length as usize])
            },
            &|| false,
        )
        .unwrap();
        for (index, block) in out.chunks(1024 * 1024).enumerate() {
            assert!(block.iter().all(|b| *b == index as u8));
        }
        let calls = std::sync::atomic::AtomicUsize::new(0);
        assert_eq!(
            transfer_chunks(
                0,
                5 * 1024 * 1024,
                &mut Vec::new(),
                &|offset, _| {
                    calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Err(if offset == 1024 * 1024 {
                        "update-access-or-release"
                    } else {
                        "update-network"
                    })
                },
                &|| false
            ),
            Err("update-access-or-release")
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 4);
    }
    #[test]
    fn full_fallback_is_once_and_cancellation_never_retries() {
        let dir = std::env::temp_dir().join(format!("fuze-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old");
        std::fs::write(&old, b"old").unwrap();
        let payload = b"new fixture package";
        let manifest = Manifest {
            schema: 1,
            channel: "rust-preview".into(),
            revision: "0123456789ab".into(),
            sequence: 2,
            archive: "Fuze-Rust-0123456789ab.fuz".into(),
            size: payload.len() as u64,
            sha512: update_io::hex(&update_io::digest(payload, true).unwrap()),
            blockmap_sha256: "00".repeat(32),
        };
        let full = Cell::new(0);
        let path = dir.join("new");
        let _ = std::fs::remove_file(&path);
        let result = fetch_with(
            &manifest,
            &path,
            Some((&old, b"invalid")),
            b"invalid",
            &|_, _| panic!("bad map must not download ranges"),
            &|out| {
                full.set(full.get() + 1);
                out.write_all(payload).unwrap();
                Ok(())
            },
            &|| false,
        )
        .unwrap();
        assert_eq!(full.get(), 1);
        assert!(result.fallback);
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        let cancel = dir.join("cancel");
        assert_eq!(
            fetch_with(
                &manifest,
                &cancel,
                None,
                b"",
                &|_, _| panic!(),
                &|_| panic!(),
                &|| true
            ),
            Err("update-cancelled")
        );
        assert!(!cancel.exists());
        let corrupt = dir.join("corrupt");
        let _ = std::fs::remove_file(&corrupt);
        assert_eq!(
            fetch_with(
                &manifest,
                &corrupt,
                None,
                b"",
                &|_, _| panic!(),
                &|out| {
                    out.write_all(&vec![0; payload.len()]).unwrap();
                    Ok(())
                },
                &|| false
            ),
            Err("update-final-hash")
        );
        for p in [old, path, corrupt] {
            std::fs::remove_file(p).unwrap();
        }
        let _ = std::fs::remove_dir(dir);
    }
}
