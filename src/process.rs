//! This process: its program's path and writing a file whole. Its
//! environment is read by kkt-machine-os; terminal behavior belongs to
//! kkt-terminal-os.

use std::path::{Path, PathBuf};

/// This program's absolute path, with links followed: the one place it is
/// found. Each caller says in its own words why it could not be.
pub fn program_path() -> std::io::Result<PathBuf> {
    std::env::current_exe().and_then(|p| p.canonicalize())
}

/// This program's absolute path, for client configs that run it.
pub fn kkt_path() -> Result<PathBuf, String> {
    program_path().map_err(|e| format!("could not find kkt's path: {e}"))
}

/// Writes `text` to `path` whole, then renames it into place, so a failure
/// leaves the earlier file.
pub fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let partial = path.with_extension("toml.partial");
    std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}
