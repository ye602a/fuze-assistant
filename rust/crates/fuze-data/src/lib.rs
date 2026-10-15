//! Read-only local preparation. No network, writes, scanning user profiles, or per-frame IO.
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
pub mod recommendations;
pub type Result<T> = std::result::Result<T, &'static str>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    ZhCn,
    ZhTw,
    EnUs,
}
impl Locale {
    pub fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::ZhTw => "zh-TW",
            Self::EnUs => "en-US",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scope {
    Ocr,
    Champions,
    Champion(u32),
    Full,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Cache,
    Bundled,
}
#[derive(Clone)]
pub struct Root {
    pub path: PathBuf,
    pub source: Source,
}
/// All loaded JSON belongs to one locale/version/root. Preparation is not activation.
pub struct Prepared {
    locale: Locale,
    version: String,
    source: Source,
    scope: Scope,
    documents: BTreeMap<String, Value>,
    catalog: HashMap<String, u32>,
}
impl Prepared {
    pub fn locale(&self) -> Locale {
        self.locale
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn source(&self) -> Source {
        self.source
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub fn identity(&self) -> String {
        format!("{}@{}", self.locale.code(), self.version)
    }
    pub fn document(&self, path: &str) -> Option<&Value> {
        self.documents.get(path)
    }
    pub fn catalog(&self) -> &HashMap<String, u32> {
        &self.catalog
    }
}
#[derive(Default)]
pub struct Active {
    data: Option<Arc<Prepared>>,
    generation: u64,
}
impl Active {
    /// Swap the already prepared snapshot once. In-flight readers retain their old Arc safely.
    pub fn activate(&mut self, next: Prepared) -> Result<u64> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or("data-generation-overflow")?;
        self.data = Some(Arc::new(next));
        self.generation = generation;
        Ok(generation)
    }
    pub fn snapshot(&self) -> Option<Arc<Prepared>> {
        self.data.clone()
    }
}
pub fn safe_path(value: &str) -> Result<&str> {
    if value.is_empty() || value.len() > 240 || value.contains('\\') {
        return Err("data-unsafe-path");
    }
    for part in value.split('/') {
        let base = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        let device = ["con", "prn", "aux", "nul"].contains(&base.as_str())
            || (base.len() == 4
                && (base.starts_with("com") || base.starts_with("lpt"))
                && matches!(base.as_bytes()[3], b'1'..=b'9'));
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.trim() != part
            || part.ends_with('.')
            || part
                .chars()
                .any(|c| c.is_control() || r#":*?"<>|"#.contains(c))
            || device
        {
            return Err("data-unsafe-path");
        }
    }
    Ok(value)
}
// Data versions currently use numeric dot components. Unsupported schemes are never guessed newer.
fn version(value: &str) -> Result<Vec<u64>> {
    if value.len() > 64 {
        return Err("data-version");
    }
    let mut parts = Vec::new();
    for part in value.split('.') {
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return Err("data-version");
        }
        parts.push(part.parse().map_err(|_| "data-version")?);
    }
    if parts.len() > 6 {
        return Err("data-version");
    }
    while parts.last() == Some(&0) && parts.len() > 1 {
        parts.pop();
    }
    Ok(parts)
}
fn read(root: &Path, relative: &str, limit: u64) -> Result<(Value, u64)> {
    safe_path(relative)?;
    let mut file = root.to_path_buf();
    for part in relative.split('/') {
        file.push(part);
        let metadata = fs::symlink_metadata(&file).map_err(|_| "data-missing")?;
        if metadata.file_type().is_symlink() {
            return Err("data-link");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("data-reparse");
            }
        }
    }
    let file = fs::canonicalize(file).map_err(|_| "data-missing")?;
    if !file.starts_with(root) {
        return Err("data-outside-root");
    }
    let mut bytes = Vec::new();
    fs::File::open(file)
        .map_err(|_| "data-read")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "data-read")?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err("data-size");
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| "data-json")?;
    Ok((value, bytes.len() as u64))
}
fn declared(value: &Value, locale: Locale, ver: &str, require_locale: bool) -> Result<()> {
    let meta = value.get("meta").unwrap_or(value);
    let lang = meta.get("locale").or_else(|| value.get("locale"));
    match lang.and_then(Value::as_str) {
        Some(lang) if lang != locale.code() => return Err("data-locale"),
        None if require_locale => return Err("data-locale"),
        _ => {}
    }
    if let Some(v) = meta.get("dataVersion").or_else(|| value.get("dataVersion")) {
        if v.as_str() != Some(ver) {
            return Err("data-version-mismatch");
        }
    }
    Ok(())
}
fn catalog(value: &Value) -> Result<HashMap<String, u32>> {
    let entries = value
        .get("data")
        .unwrap_or(value)
        .as_array()
        .ok_or("data-augment-shape")?;
    if entries.is_empty() || entries.len() > 10000 {
        return Err("data-augment-count");
    }
    let mut result = HashMap::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
            .filter(|id| *id > 0 && *id <= u32::MAX as u64)
            .ok_or("data-augment-id")? as u32;
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .ok_or("data-augment-name")?;
        if name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("data-augment-name");
        }
        let name: String = name.chars().filter(|c| !c.is_whitespace()).collect();
        if name.is_empty() {
            return Err("data-augment-name");
        }
        if result.insert(name, id).is_some_and(|old| old != id) {
            return Err("data-ambiguous-name");
        }
    }
    Ok(result)
}
fn required(manifest: &Value, scope: Scope) -> Result<BTreeMap<String, u64>> {
    let mut files = BTreeMap::from([("augments.json".into(), 0)]);
    if scope == Scope::Champions {
        files.insert("champions.json".into(), 0);
    }
    if scope == Scope::Full {
        for p in ["champions.json", "items.json", "champion-shards/index.json"] {
            files.insert(p.into(), 0);
        }
    }
    if matches!(scope, Scope::Champion(_)) {
        files.insert("champion-shards/index.json".into(), 0);
    }
    let mut add = |path: &str, entry: &Value| -> Result<()> {
        safe_path(path)?;
        if path == "augments.json"
            || scope == Scope::Champions && path == "champions.json"
            || scope == Scope::Full
                && (["champions.json", "items.json"].contains(&path)
                    || path.starts_with("champion-shards/"))
        {
            let bytes = entry.get("bytes").and_then(Value::as_u64).unwrap_or(0);
            if bytes > 32 * 1024 * 1024 {
                return Err("data-size");
            }
            files.insert(path.into(), bytes);
        }
        Ok(())
    };
    match manifest.get("files") {
        Some(Value::Array(entries)) if entries.len() <= 1024 => {
            for entry in entries {
                let path = entry
                    .get("path")
                    .or_else(|| entry.get("logicalPath"))
                    .and_then(Value::as_str)
                    .ok_or("data-manifest-path")?;
                add(path, entry)?;
            }
        }
        Some(Value::Object(entries)) if entries.len() <= 1024 => {
            for (path, entry) in entries {
                add(path, entry)?;
            }
        }
        _ => return Err("data-manifest-files"),
    }
    if files.len() > 512 {
        return Err("data-file-count");
    }
    Ok(files)
}
fn load(
    root: &Path,
    prefix: &str,
    locale: Locale,
    ver: &str,
    source: Source,
    scope: Scope,
) -> Result<Prepared> {
    let (manifest, _) = read(root, &format!("{prefix}/manifest.json"), 2 * 1024 * 1024)?;
    declared(&manifest, locale, ver, locale != Locale::ZhCn)?;
    let files = required(&manifest, scope)?;
    let mut documents = BTreeMap::new();
    let mut total = 0;
    for (path, expected) in files {
        let (value, size) = read(root, &format!("{prefix}/{path}"), 32 * 1024 * 1024)?;
        total += size;
        if total > 128 * 1024 * 1024 || expected > 0 && size != expected {
            return Err("data-size");
        }
        declared(&value, locale, ver, false)?;
        documents.insert(path, value);
    }
    if let Scope::Champion(champion) = scope {
        let index = documents
            .get("champion-shards/index.json")
            .ok_or("data-shard-index")?;
        let entries = index
            .get("shards")
            .and_then(Value::as_array)
            .ok_or("data-shard-index")?;
        if entries.len() > 512 {
            return Err("data-file-count");
        }
        let mut selected = None;
        for entry in entries {
            if entry
                .get("championIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_u64() == Some(champion as u64)))
            {
                if selected.is_some() {
                    return Err("data-ambiguous-shard");
                }
                selected = entry.get("path").and_then(Value::as_str);
            }
        }
        let path = selected.ok_or("data-champion-missing")?;
        if !path.starts_with("champion-shards/") || path == "champion-shards/index.json" {
            return Err("data-shard-path");
        }
        let (value, _) = read(root, &format!("{prefix}/{path}"), 32 * 1024 * 1024)?;
        declared(&value, locale, ver, false)?;
        documents.insert(path.into(), value);
    }
    let catalog = catalog(
        documents
            .get("augments.json")
            .ok_or("data-augments-missing")?,
    )?;
    documents.insert("manifest.json".into(), manifest);
    Ok(Prepared {
        locale,
        version: ver.into(),
        source,
        scope,
        documents,
        catalog,
    })
}
/// Roots are explicitly supplied by the host. No disk scan, automatic directory creation or network.
pub fn prepare(roots: &[Root], locale: Locale, scope: Scope) -> Result<Prepared> {
    if roots.len() > 8 {
        return Err("data-root-count");
    }
    let mut candidates = Vec::new();
    for root in roots {
        let Ok(path) = fs::canonicalize(&root.path) else {
            continue;
        };
        let names = if locale == Locale::ZhCn {
            vec!["current.json".into(), "current.zh-CN.json".into()]
        } else {
            vec![format!("current.{}.json", locale.code())]
        };
        for name in names {
            let Ok((pointer, _)) = read(&path, &name, 65536) else {
                continue;
            };
            if pointer.get("schemaVersion").and_then(Value::as_u64) != Some(3) {
                continue;
            }
            let Some(ver) = pointer.get("dataVersion").and_then(Value::as_str) else {
                continue;
            };
            let Ok(order) = version(ver) else { continue };
            if declared(&pointer, locale, ver, locale != Locale::ZhCn).is_err() {
                continue;
            }
            candidates.push((order, path.clone(), root.source, ver.to_string()));
        }
    }
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| (a.2 != Source::Cache).cmp(&(b.2 != Source::Cache)))
    });
    for (_, root, source, ver) in candidates {
        let mut prefixes = vec![format!("versions/{}/{ver}", locale.code())];
        if locale == Locale::ZhCn {
            prefixes.push(format!("versions/{ver}"));
        }
        for prefix in prefixes {
            if let Ok(data) = load(&root, &prefix, locale, &ver, source, scope) {
                return Ok(data);
            }
        }
    }
    Err("data-no-complete-local-version")
}
