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
        || !matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("json" | "png")
        )
        || request.bytes.len() > 4 * 1024 * 1024
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
    let filter: Vec<u16> = "JSON\0*.json\0PNG\0*.png\0\0".encode_utf16().collect();
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
    let mut path = Path::new(&String::from_utf16_lossy(&file[..n])).to_owned();
    if dialog.nFilterIndex == 2 {
        path.set_extension("png");
    }
    Some(path)
}

/// Render a fresh memory bitmap from the reduced export. Never reads pixels from any window.
pub fn png(
    bytes: &[u8],
    public_detail: Option<&fuze_data::champion_detail::Detail>,
    pictures: Option<&crate::champion_assets::Reply>,
) -> Result<Vec<u8>> {
    use crate::desktop::{fill, rgb, text};
    use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*};
    let data: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| "export-encode")?;
    if data["kind"] != "fuze-anonymous-match-summary" {
        return Err("export-invalid-result");
    }
    let r = &data["result"];
    let t = crate::i18n::text;
    let hero = public_detail
        .map(|d| d.title.clone())
        .unwrap_or_else(|| format!("#{}", r["champion"].as_u64().unwrap_or(0)));
    let names = |index: usize, fallback: &serde_json::Value| {
        public_detail
            .and_then(|d| d.lines.get(index))
            .map(|l| l.text.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ids(fallback))
    };
    let lines = [
        t("FUZE / 对局分享").to_string(),
        format!(
            "{}     {}",
            hero,
            match r["outcome"].as_str() {
                Some("win") => t("胜利"),
                Some("loss") => t("失败"),
                _ => t("未知"),
            }
        ),
        format!(
            "{} / {} / {}     K / D / A",
            r["kills"].as_u64().unwrap_or(0),
            r["deaths"].as_u64().unwrap_or(0),
            r["assists"].as_u64().unwrap_or(0)
        ),
        format!(
            "{} {}:{:02}    {}",
            t("时长"),
            r["durationSeconds"].as_u64().unwrap_or(0) / 60,
            r["durationSeconds"].as_u64().unwrap_or(0) % 60,
            if data["cached"] == true {
                t("缓存结果")
            } else {
                t("对局结果")
            }
        ),
        format!("{}  {}", t("装备"), names(1, &r["items"])),
        format!("{}  {}", t("结算强化"), names(2, &r["augments"])),
        t("领取顺序未确认 / 推荐使用整体胜率").into(),
        t("未包含候选观察、账号、区服、对局编号或时间戳。").into(),
    ];
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return Err("export-image");
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 1200,
            biHeight: -980,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut pixels = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut pixels,
            std::ptr::null_mut(),
            0,
        );
        if bitmap.is_null() {
            DeleteDC(dc);
            return Err("export-image");
        }
        let old = SelectObject(dc, bitmap);
        fill(
            dc,
            RECT {
                left: 0,
                top: 0,
                right: 1200,
                bottom: 980,
            },
            rgb(6, 19, 29),
        );
        fill(
            dc,
            RECT {
                left: 36,
                top: 30,
                right: 42,
                bottom: 945,
            },
            rgb(195, 164, 94),
        );
        for (i, line) in lines.iter().enumerate().filter(|(i, _)| *i < 4 || *i >= 6) {
            let top = [32, 104, 158, 218, 0, 0, 886, 924][i];
            text(
                dc,
                line,
                RECT {
                    left: if i == 1 || i == 2 { 190 } else { 62 },
                    top,
                    right: 1170,
                    bottom: top + 50,
                },
                if i < 3 { 30 } else { 19 },
                if i == 0 {
                    rgb(221, 190, 118)
                } else {
                    rgb(179, 214, 216)
                },
            );
        }
        let empty = crate::champion_assets::Reply::default();
        let pictures = pictures.unwrap_or(&empty);
        if let Some(url) = public_detail
            .and_then(|d| d.lines.first())
            .and_then(|l| l.icons.first())
        {
            crate::champ_select::draw_icon(dc, pictures, url, 62, 104, 100);
        } else {
            crate::champ_select::draw_icon(dc, pictures, "", 62, 104, 100);
        }
        for (section, key, label, top) in
            [(1, "items", "装备", 282), (2, "augments", "结算强化", 578)]
        {
            text(
                dc,
                label,
                RECT {
                    left: 62,
                    top,
                    right: 1160,
                    bottom: top + 34,
                },
                24,
                rgb(221, 190, 118),
            );
            let entries = r[key].as_array().map(|v| v.as_slice()).unwrap_or(&[]);
            let public_line = public_detail.and_then(|d| d.lines.get(section));
            let labels = public_line
                .map(|l| l.text.split(" · ").collect::<Vec<_>>())
                .unwrap_or_default();
            if entries.is_empty() {
                text(
                    dc,
                    "--",
                    RECT {
                        left: 62,
                        top: top + 50,
                        right: 300,
                        bottom: top + 90,
                    },
                    24,
                    rgb(138, 167, 180),
                );
            }
            for (index, id) in entries.iter().take(12).enumerate() {
                let left = 62 + (index % 6) as i32 * 184;
                let y = top + 44 + (index / 6) as i32 * 116;
                fill(
                    dc,
                    RECT {
                        left,
                        top: y,
                        right: left + 174,
                        bottom: y + 108,
                    },
                    rgb(16, 35, 46),
                );
                let url = public_line
                    .and_then(|l| l.icons.get(index))
                    .map(String::as_str)
                    .unwrap_or("");
                crate::champ_select::draw_icon(dc, pictures, url, left + 61, y + 6, 52);
                let fallback = format!("#{}", id.as_u64().unwrap_or(0));
                let name = if labels.len() == entries.len() {
                    labels[index]
                } else {
                    &fallback
                };
                let columns = if crate::i18n::locale() == "en-US" {
                    17
                } else {
                    10
                };
                let chars: Vec<_> = name.chars().take(160).collect();
                for (n, chunk) in chars.chunks(columns).take(2).enumerate() {
                    let mut line: String = chunk.iter().collect();
                    if n == 1 && chars.len() > columns * 2 {
                        line.pop();
                        line.push('…');
                    }
                    text(
                        dc,
                        &line,
                        RECT {
                            left: left + 8,
                            top: y + 61 + n as i32 * 20,
                            right: left + 166,
                            bottom: y + 81 + n as i32 * 20,
                        },
                        15,
                        rgb(179, 214, 216),
                    );
                }
            }
        }
        GdiFlush();
        let mut rgba = std::slice::from_raw_parts(pixels as *const u8, 1200 * 980 * 4).to_vec();
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        let image = image::RgbaImage::from_raw(1200, 980, rgba).ok_or("export-image")?;
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|_| "export-image")?;
        Ok(output.into_inner())
    }
}
fn ids(value: &serde_json::Value) -> String {
    value
        .as_array()
        .map(|v| {
            v.iter()
                .take(12)
                .filter_map(|v| v.as_u64())
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "--".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn share_icons_are_memory_only_and_maximum_slots_fit_all_locales() {
        use fuze_data::champion_detail::{Detail, Line};
        let row = fuze_lcu::history::Match {
            game_id: 987654321,
            created: 123456789,
            duration: 1620,
            champion: 21,
            kills: 16,
            deaths: 5,
            assists: 28,
            result: "win".into(),
            items: (1001..1013).collect(),
            augments: (1..13).collect(),
        };
        let bytes = encode(&row, None, true).unwrap();
        let mut pictures = crate::champion_assets::Reply::default();
        let urls: Vec<_> = (0..13).map(|i| format!("fixture:{i}")).collect();
        for (i, url) in urls.iter().enumerate() {
            let mut bgra = vec![0u8; 48 * 48 * 4];
            for y in 0..48 {
                for x in 0..48 {
                    let offset = (y * 48 + x) * 4;
                    let bright = (x as i32 - 24).abs() + (y as i32 - 24).abs() < 18;
                    let color = if bright {
                        [90, 190, 225, 255]
                    } else {
                        [70 + i as u8 * 7, 45, 20, 255]
                    };
                    bgra[offset..offset + 4].copy_from_slice(&color);
                }
            }
            pictures.icons.insert(
                url.clone(),
                std::sync::Arc::new(crate::champion_assets::Picture { bgra }),
            );
        }
        for locale in ["zh-CN", "zh-TW", "en-US"] {
            crate::i18n::set(locale);
            let name = if locale == "en-US" {
                "Long synthetic augment name"
            } else {
                "合成强化名称用于检查换行"
            };
            let names = (1..=12)
                .map(|i| format!("{name} {i}"))
                .collect::<Vec<_>>()
                .join(" · ");
            let detail = Detail {
                title: if locale == "en-US" {
                    "Synthetic champion".into()
                } else {
                    "合成英雄 · 测试数据".into()
                },
                lines: vec![
                    Line {
                        text: String::new(),
                        icons: vec![urls[0].clone()],
                    },
                    Line {
                        text: names.clone(),
                        icons: urls[1..].to_vec(),
                    },
                    Line {
                        text: names,
                        icons: urls[1..].to_vec(),
                    },
                ],
            };
            let png = png(&bytes, Some(&detail), Some(&pictures)).unwrap();
            let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
            assert_eq!(decoded.dimensions(), (1200, 980));
            assert_ne!(decoded.get_pixel(112, 154), decoded.get_pixel(0, 0));
            assert_ne!(
                decoded.get_pixel(62 + 5 * 184 + 80, 578 + 44 + 116 + 30),
                decoded.get_pixel(0, 0)
            );
            if let Some(path) = std::env::var_os("FUZE_SHARE_FIXTURE") {
                save(Request {
                    path: PathBuf::from(path).with_file_name(format!("share-icons-{locale}.png")),
                    bytes: png,
                })
                .unwrap();
            }
        }
        crate::i18n::set("zh-CN");
        assert_eq!(pictures.icons.len(), 13);
    }
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
        let picture = png(&bytes, None, None).unwrap();
        assert!(picture.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(image::load_from_memory(&picture).unwrap().width(), 1200);
        if let Some(path) = std::env::var_os("FUZE_SHARE_FIXTURE") {
            save(Request {
                path: path.into(),
                bytes: picture,
            })
            .unwrap();
        }
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
