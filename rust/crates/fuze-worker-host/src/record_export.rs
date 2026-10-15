//! Explicit local export of a reduced snapshot. No account, region, timestamps or match IDs.
use crate::Result;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
pub struct Request {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}
pub fn encode(
    row: &fuze_lcu::history::Match,
    observation: Option<&fuze_lcu::history::Observation>,
    cached: bool,
) -> Result<Vec<u8>> {
    if !row.valid() {
        return Err("export-invalid-result");
    }
    let observation = observation.filter(|o| o.game_id == row.game_id && o.valid());
    let data = serde_json::json!({
        "schema":1,"kind":"fuze-anonymous-match-summary","cached":cached,
        "result":{"champion":row.champion,"outcome":row.result,"durationSeconds":row.duration,
            "kills":row.kills,"deaths":row.deaths,"assists":row.assists,"items":row.items,"augments":row.augments},
        "candidateObservations":observation.map(|o| &o.events),
        "observationsInterrupted":observation.map(|o| o.interrupted),
        "observationsCapped":observation.map(|o| o.capped),
        "actualClaimRound":null,"recommendationScope":"overall",
        "note":"Candidate observations are not claims. Result augments have no inferred acquisition order."
    });
    let bytes = serde_json::to_vec_pretty(&data).map_err(|_| "export-encode")?;
    if bytes.len() > 65536 {
        return Err("export-limit");
    }
    Ok(bytes)
}
pub fn save(request: Request) -> Result<()> {
    let path = request.path;
    if !path.is_absolute()
        || path.to_string_lossy().starts_with("\\\\")
        || path.extension().and_then(|s| s.to_str()) != Some("json")
        || request.bytes.len() > 65536
    {
        return Err("export-path");
    }
    let parent = path.parent().ok_or("export-path")?;
    if !parent.is_dir() {
        return Err("export-path");
    }
    crate::managed_install::safe_dir(parent)?;
    // Never replace an existing file, including a symlink or previous user export.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "export-create-new")?;
    if file
        .write_all(&request.bytes)
        .and_then(|_| file.sync_all())
        .is_err()
    {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err("export-write");
    }
    Ok(())
}
pub unsafe fn choose(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<PathBuf> {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut file = [0u16; 32768];
    let filter: Vec<u16> = "JSON\0*.json\0\0".encode_utf16().collect();
    let extension: Vec<u16> = "json\0".encode_utf16().collect();
    let mut dialog: OPENFILENAMEW = std::mem::zeroed();
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = hwnd;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrDefExt = extension.as_ptr();
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.Flags = OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_DONTADDTORECENT;
    if GetSaveFileNameW(&mut dialog) == 0 {
        return None;
    }
    let n = file.iter().position(|c| *c == 0)?;
    Some(Path::new(&String::from_utf16_lossy(&file[..n])).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reduced_export_keeps_partial_slots_without_identity_or_claim_inference() {
        let row = fuze_lcu::history::Match {
            game_id: 99887766,
            created: 1234567890,
            duration: 900,
            champion: 21,
            kills: 3,
            deaths: 2,
            assists: 1,
            result: "win".into(),
            items: vec![1001],
            augments: vec![7],
        };
        let observation = fuze_lcu::history::Observation {
            game_id: row.game_id,
            interrupted: true,
            capped: false,
            events: vec![fuze_lcu::history::ObservedCards {
                cards: [Some(1), None, Some(3)],
                level: Some(7),
                partial: true,
                kind: "changed".into(),
            }],
        };
        let bytes = encode(&row, Some(&observation), true).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        for forbidden in [
            "99887766",
            "1234567890",
            "game_id",
            "created",
            "account",
            "platform",
            "puuid",
        ] {
            assert!(!text.contains(forbidden));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["actualClaimRound"].is_null());
        assert!(value["candidateObservations"][0]["cards"][1].is_null());
        assert_eq!(value["result"]["augments"], serde_json::json!([7]));
        let path =
            std::env::temp_dir().join(format!("fuze-export-fixture-{}.json", std::process::id()));
        save(Request {
            path: path.clone(),
            bytes: bytes.clone(),
        })
        .unwrap();
        assert_eq!(
            save(Request {
                path: path.clone(),
                bytes: vec![]
            }),
            Err("export-create-new")
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path).unwrap();
    }
}
