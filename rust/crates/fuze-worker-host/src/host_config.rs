//! Shared bounded configuration loader. No discovery, worker launch or writes.
use crate::{controller::Config, Result};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
pub fn load(path: &Path) -> Result<Config> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        bundled_data: PathBuf,
        cache_data: Option<PathBuf>,
        locale: String,
        worker: PathBuf,
        ort: PathBuf,
        model: PathBuf,
        dictionary: PathBuf,
        backend: String,
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "config-read")?
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(|_| "config-read")?;
    if bytes.len() > 16384 {
        return Err("config-limit");
    }
    let input: Input = serde_json::from_slice(&bytes).map_err(|_| "config-invalid")?;
    let config_path = std::fs::canonicalize(path).map_err(|_| "config-read")?;
    let directory = config_path.parent().ok_or("config-path")?;
    let resolve = |path: PathBuf| {
        if path.is_absolute() {
            path
        } else {
            directory.join(path)
        }
    };
    let locale = match input.locale.as_str() {
        "auto" => None,
        "zh-CN" => Some(fuze_data::Locale::ZhCn),
        "zh-TW" => Some(fuze_data::Locale::ZhTw),
        "en-US" => Some(fuze_data::Locale::EnUs),
        _ => return Err("locale-invalid"),
    };
    if !["cpu", "auto", "dml", "dml-strict"].contains(&input.backend.as_str()) {
        return Err("backend-invalid");
    }
    let mut roots = vec![fuze_data::Root {
        path: resolve(input.bundled_data),
        source: fuze_data::Source::Bundled,
    }];
    if let Some(path) = input.cache_data {
        roots.push(fuze_data::Root {
            path: resolve(path),
            source: fuze_data::Source::Cache,
        });
    }
    Ok(Config {
        icon_downloads: false,
        level_gate: true,
        augment_overlay: true,
        roots,
        locale,
        worker: resolve(input.worker),
        ort: resolve(input.ort),
        model: resolve(input.model),
        dictionary: resolve(input.dictionary),
        backend: input.backend,
    })
}
