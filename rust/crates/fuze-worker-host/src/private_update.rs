//! Bounded differential reconstruction, shared by explicit public and private updates.
use crate::{update_io, Result};
use serde::Deserialize;
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom, Write},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Map {
    version: String,
    files: Vec<Blocks>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Blocks {
    name: String,
    offset: u64,
    sizes: Vec<u64>,
    checksums: Vec<String>,
}
fn parse(bytes: &[u8], size: u64) -> Result<Blocks> {
    if bytes.len() > 8 * 1024 * 1024 || size == 0 || size > 512 * 1024 * 1024 {
        return Err("delta-map-limit");
    }
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut decoded)
        .map_err(|_| "delta-map-gzip")?;
    if decoded.len() > 8 * 1024 * 1024 {
        return Err("delta-map-limit");
    }
    let mut map: Map = serde_json::from_slice(&decoded).map_err(|_| "delta-map-format")?;
    if map.version != "2" || map.files.len() != 1 {
        return Err("delta-map-format");
    }
    let file = map.files.pop().ok_or("delta-map-format")?;
    if file.name != "file"
        || file.offset != 0
        || file.sizes.is_empty()
        || file.sizes.len() > 100000
        || file.sizes.len() != file.checksums.len()
        || file.sizes.iter().any(|s| *s == 0)
        || file.checksums.iter().any(|s| s.is_empty() || s.len() > 128)
        || file
            .sizes
            .iter()
            .try_fold(0u64, |sum, v| sum.checked_add(*v))
            != Some(size)
    {
        return Err("delta-map-format");
    }
    Ok(file)
}
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Copy { offset: u64, length: u64 },
    Download { offset: u64, length: u64 },
}
pub fn plan(
    old_map: &[u8],
    old_size: u64,
    new_map: &[u8],
    new_size: u64,
) -> Result<Vec<Operation>> {
    let old = parse(old_map, old_size)?;
    let new = parse(new_map, new_size)?;
    let mut known = HashMap::new();
    let mut offset = 0;
    for (size, checksum) in old.sizes.iter().zip(old.checksums.iter()) {
        known.entry((*size, checksum)).or_insert(offset);
        offset += size;
    }
    let mut out = Vec::with_capacity(new.sizes.len());
    offset = 0;
    for (size, checksum) in new.sizes.iter().zip(new.checksums.iter()) {
        out.push(if let Some(start) = known.get(&(*size, checksum)) {
            Operation::Copy {
                offset: *start,
                length: *size,
            }
        } else {
            Operation::Download {
                offset,
                length: *size,
            }
        });
        offset += size;
    }
    let mut merged: Vec<Operation> = Vec::new();
    for op in out {
        let extends = match (merged.last_mut(), &op) {
            (
                Some(Operation::Copy {
                    offset: a,
                    length: b,
                }),
                Operation::Copy {
                    offset: c,
                    length: d,
                },
            )
            | (
                Some(Operation::Download {
                    offset: a,
                    length: b,
                }),
                Operation::Download {
                    offset: c,
                    length: d,
                },
            ) if *a + *b == *c => {
                *b += *d;
                true
            }
            _ => false,
        };
        if !extends {
            merged.push(op);
        }
    }
    Ok(merged)
}
/// Exact range lengths are checked again after the HTTPS transport validates 206/Content-Range.
/// Output must be a new staging file. Final SHA-512 is required before any caller activates it.
pub fn reconstruct<R: Read + Seek, W: Read + Write + Seek>(
    old: &mut R,
    out: &mut W,
    operations: &[Operation],
    size: u64,
    expected_sha512: &[u8],
    fetch: &dyn Fn(u64, u64) -> Result<Vec<u8>>,
    cancel: &dyn Fn() -> bool,
) -> Result<u64> {
    reconstruct_observed(
        old,
        out,
        operations,
        size,
        expected_sha512,
        fetch,
        cancel,
        &|| {},
    )
}
pub fn reconstruct_observed<R: Read + Seek, W: Read + Write + Seek>(
    old: &mut R,
    out: &mut W,
    operations: &[Operation],
    size: u64,
    expected_sha512: &[u8],
    fetch: &dyn Fn(u64, u64) -> Result<Vec<u8>>,
    cancel: &dyn Fn() -> bool,
    before_verify: &dyn Fn(),
) -> Result<u64> {
    if size == 0
        || size > 512 * 1024 * 1024
        || expected_sha512.len() != 64
        || operations.len() > 100000
    {
        return Err("delta-size");
    }
    out.seek(SeekFrom::Start(0)).map_err(|_| "delta-write")?;
    let mut written = 0u64;
    let mut downloaded = 0u64;
    for operation in operations {
        let (offset, length, copy) = match operation {
            Operation::Copy { offset, length } => (*offset, *length, true),
            Operation::Download { offset, length } => (*offset, *length, false),
        };
        if length == 0
            || written.checked_add(length).filter(|n| *n <= size).is_none()
            || offset.checked_add(length).is_none()
        {
            return Err("delta-size");
        }
        if !copy && offset != written {
            return Err("delta-range");
        }
        if copy {
            old.seek(SeekFrom::Start(offset))
                .map_err(|_| "delta-baseline")?;
        }
        let mut consumed = 0;
        while consumed < length {
            if cancel() {
                return Err("delta-cancelled");
            }
            let count = (length - consumed).min(if copy { 65536 } else { 1024 * 1024 }) as usize;
            let bytes = if copy {
                let mut b = vec![0; count];
                old.read_exact(&mut b).map_err(|_| "delta-baseline")?;
                b
            } else {
                let b = fetch(offset + consumed, count as u64)?;
                if b.len() != count {
                    return Err("delta-range");
                }
                downloaded += count as u64;
                b
            };
            out.write_all(&bytes).map_err(|_| "delta-write")?;
            consumed += count as u64;
        }
        written += length;
    }
    if written != size {
        return Err("delta-size");
    }
    out.flush().map_err(|_| "delta-write")?;
    if out.seek(SeekFrom::End(0)).map_err(|_| "delta-write")? != size {
        return Err("delta-size");
    }
    out.seek(SeekFrom::Start(0)).map_err(|_| "delta-write")?;
    // BCrypt streaming hash: only one 64 KiB buffer, never the whole installer in memory.
    before_verify();
    let actual = hash_reader(out, cancel)?;
    if actual != expected_sha512 {
        return Err("delta-final-hash");
    }
    Ok(downloaded)
}
pub(crate) fn hash_reader(reader: &mut dyn Read, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    hash_reader_with_algorithm(reader, cancel, true)
}
/// Shared bounded-buffer hashing for update archives and native prerequisite checks.
pub(crate) fn hash_reader_with_algorithm(
    reader: &mut dyn Read,
    cancel: &dyn Fn() -> bool,
    sha512: bool,
) -> Result<Vec<u8>> {
    use windows_sys::Win32::Security::Cryptography::*;
    struct Hash(BCRYPT_HASH_HANDLE);
    impl Drop for Hash {
        fn drop(&mut self) {
            unsafe {
                BCryptDestroyHash(self.0);
            }
        }
    }
    unsafe {
        let mut raw = std::ptr::null_mut();
        if BCryptCreateHash(
            if sha512 {
                BCRYPT_SHA512_ALG_HANDLE
            } else {
                BCRYPT_SHA256_ALG_HANDLE
            },
            &mut raw,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            0,
            0,
        ) < 0
        {
            return Err("delta-hash");
        }
        let hash = Hash(raw);
        let mut buf = [0u8; 65536];
        loop {
            if cancel() {
                return Err("delta-cancelled");
            }
            let n = reader.read(&mut buf).map_err(|_| "delta-read")?;
            if n == 0 {
                break;
            }
            if BCryptHashData(hash.0, buf.as_ptr(), n as u32, 0) < 0 {
                return Err("delta-hash");
            }
        }
        let mut out = vec![0; if sha512 { 64 } else { 32 }];
        if BCryptFinishHash(hash.0, out.as_mut_ptr(), out.len() as u32, 0) < 0 {
            return Err("delta-hash");
        }
        Ok(out)
    }
}
#[derive(Debug, PartialEq)]
pub enum Target<'a> {
    Api(&'a str),
    Cdn { host: &'a str, path: &'a str },
}
pub fn target(url: &str) -> Result<Target<'_>> {
    if url.len() > 8192 || url.chars().any(|c| c.is_control() || "\\#".contains(c)) {
        return Err("private-url");
    }
    let rest = url.strip_prefix("https://").ok_or("private-url")?;
    let (host, _) = rest.split_once('/').ok_or("private-url")?;
    let path = &rest[host.len()..];
    if host == "api.github.com" {
        let suffix = path
            .strip_prefix("/repos/ye602a/fuze-assistant/releases/")
            .ok_or("private-url")?;
        let asset = suffix
            .strip_prefix("assets/")
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()));
        if suffix == "latest" || suffix == "tags/rust-preview" || asset {
            return Ok(Target::Api(path));
        }
    } else if [
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
    ]
    .contains(&host)
    {
        return Ok(Target::Cdn { host, path });
    }
    Err("private-url")
}
/// Tokens are accepted only for the fixed repository API; a CDN never receives Authorization.
/// Redirect following is deliberately not exposed until a native release manifest is defined.
pub fn range(
    url: &str,
    token: Option<&str>,
    offset: u64,
    length: u64,
    total: u64,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    if length == 0
        || length > 1024 * 1024
        || offset
            .checked_add(length)
            .filter(|end| *end <= total)
            .is_none()
    {
        return Err("delta-range");
    }
    let (host, path, auth) = match target(url)? {
        Target::Api(path) => ("api.github.com", path, token),
        Target::Cdn { host, path } => (host, path, None),
    };
    update_io::get(
        host,
        path,
        length as usize,
        auth,
        Some((offset, offset + length - 1, total)),
        cancel,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn streaming_sha256_matches_known_digest_and_both_algorithms_bound_reads() {
        let hash = hash_reader_with_algorithm(&mut Cursor::new(b"abc"), &|| false, false).unwrap();
        assert_eq!(
            update_io::hex(&hash),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        struct Chunks(usize);
        impl Read for Chunks {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                assert!(bytes.len() <= 65536);
                // Deliberately short reads across several buffer boundaries.
                let n = self.0.min(bytes.len()).min(8191);
                bytes[..n].fill(7);
                self.0 -= n;
                Ok(n)
            }
        }
        for sha512 in [false, true] {
            let expected = update_io::digest(&vec![7; 196_621], sha512).unwrap();
            assert_eq!(
                hash_reader_with_algorithm(&mut Chunks(196_621), &|| false, sha512).unwrap(),
                expected
            );
        }
    }
    #[test]
    fn streaming_hash_keeps_cancellation_and_read_errors() {
        struct Failed;
        impl Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::PermissionDenied.into())
            }
        }
        for sha512 in [false, true] {
            assert_eq!(
                hash_reader_with_algorithm(&mut Failed, &|| false, sha512),
                Err("delta-read")
            );
            assert_eq!(
                hash_reader_with_algorithm(&mut Failed, &|| true, sha512),
                Err("delta-cancelled")
            );
        }
    }
    fn map(sizes: &[u64], checksums: &[&str]) -> Vec<u8> {
        let v = serde_json::json!({"version":"2","files":[{"name":"file","offset":0,"sizes":sizes,"checksums":checksums}]});
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(v.to_string().as_bytes()).unwrap();
        gzip.finish().unwrap()
    }
    #[test]
    fn private_policy_and_reconstruction_reject_corruption_and_cancellation() {
        assert!(matches!(
            target("https://api.github.com/repos/ye602a/fuze-assistant/releases/assets/7"),
            Ok(Target::Api(_))
        ));
        for bad in [
            "http://api.github.com/repos/ye602a/fuze-assistant/releases/latest",
            "https://api.github.com.evil/releases/latest",
            "https://token@api.github.com/repos/ye602a/fuze-assistant/releases/latest",
            "https://api.github.com/repos/other/repo/releases/latest",
        ] {
            assert!(target(bad).is_err());
        }
        let p = plan(&map(&[4, 4], &["a", "b"]), 8, &map(&[4, 4], &["b", "c"]), 8).unwrap();
        assert_eq!(
            p,
            vec![
                Operation::Copy {
                    offset: 4,
                    length: 4
                },
                Operation::Download {
                    offset: 4,
                    length: 4
                }
            ]
        );
        let hash = update_io::digest(b"BBBBCCCC", true).unwrap();
        let mut out = Cursor::new(Vec::new());
        assert_eq!(
            reconstruct(
                &mut Cursor::new(b"AAAABBBB"),
                &mut out,
                &p,
                8,
                &hash,
                &|offset, len| {
                    assert_eq!((offset, len), (4, 4));
                    Ok(b"CCCC".to_vec())
                },
                &|| false
            ),
            Ok(4)
        );
        assert_eq!(out.into_inner(), b"BBBBCCCC");
        assert_eq!(
            reconstruct(
                &mut Cursor::new(b"AAAAXXXX"),
                &mut Cursor::new(Vec::new()),
                &p,
                8,
                &hash,
                &|_, _| Ok(b"CCCC".to_vec()),
                &|| false
            ),
            Err("delta-final-hash")
        );
        assert_eq!(
            reconstruct(
                &mut Cursor::new(b"AAAABBBB"),
                &mut Cursor::new(Vec::new()),
                &p,
                8,
                &hash,
                &|_, _| Ok(b"CCCC".to_vec()),
                &|| true
            ),
            Err("delta-cancelled")
        );
        assert!(plan(&map(&[4, 4], &["a"]), 8, &map(&[8], &["b"]), 8).is_err());
    }
}
