//! Explicit developer-only comparison. No API requests, raw output, copied logs or stored secrets.
//! Set FUZE_LCU_LOG_REVIEW_DIR yourself and run only this ignored test with --nocapture.
use super::*;

#[test]
#[ignore = "requires explicitly selected local logs; never runs in CI or at app startup"]
fn local_log_decoder_comparison() {
    let root = std::path::PathBuf::from(
        std::env::var_os("FUZE_LCU_LOG_REVIEW_DIR")
            .expect("explicit local review directory required"),
    );
    let mut candidates = Vec::new();
    for dir in [
        root.clone(),
        root.join("LeagueClient"),
        root.join("Logs/LeagueClient Logs"),
        root.join("Logs"),
    ] {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.take(4096).flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(prefix) = name.strip_suffix("_LeagueClientUx.log") else {
                continue;
            };
            let Some(pid) = prefix
                .rsplit('_')
                .next()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_file() {
                candidates.push((
                    meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                    entry.path(),
                    pid,
                ));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total = 0;
    let mut recovered = 0;
    for (_, path, pid) in candidates.into_iter().take(5) {
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        let mut buffer = Secret(Vec::with_capacity(MAX_BYTES));
        if file
            .take(MAX_BYTES as u64)
            .read_to_end(&mut buffer.0)
            .is_err()
        {
            continue;
        }
        let prefix = match std::str::from_utf8(&buffer.0) {
            Ok(text) => text,
            Err(e) => std::str::from_utf8(&buffer.0[..e.valid_up_to()]).unwrap_or(""),
        };
        let old = parse(pid, prefix).is_ok();
        let new = parse_bytes(pid, &buffer.0).is_ok();
        let file_route = credentials(&root, pid, None).is_ok();
        total += 1;
        recovered += usize::from(!old && new && file_route);
        println!("sample={total} old_decoder={old} new_decoder={new} file_route={file_route}");
    }
    println!("samples={total} recovered={recovered} game_access=false raw_output=false");
    assert!(
        total > 0 && recovered > 0,
        "no encoding regression reproduced in selected samples"
    );
}
