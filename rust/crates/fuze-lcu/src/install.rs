//! Shared, bounded directory layout support for preferences and LCU discovery.
use std::path::{Path, PathBuf};

pub fn client_directories(root: &Path) -> Vec<PathBuf> {
    [root.to_path_buf(), root.join("LeagueClient")]
        .into_iter()
        .filter(|dir| {
            dir.join("LeagueClient.exe").is_file() || dir.join("LeagueClientUx.exe").is_file()
        })
        .collect()
}

pub fn is_client_directory(root: &Path) -> bool {
    !client_directories(root).is_empty()
}
