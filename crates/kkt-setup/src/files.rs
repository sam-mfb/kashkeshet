//! Setup's record and unfinished note, data folder and program paths on
//! this machine, behind `real`.

use crate::{Record, Unfinished};
use std::path::{Path, PathBuf};

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

/// Reads the setup record from the selected config folder, if present.
pub fn read_record(config_dir: &Path) -> Result<Option<Record>, String> {
    let path = crate::record_path(config_dir);
    read_optional(&path)?
        .map(|text| Record::parse(&text, &path))
        .transpose()
}

/// Reads the unfinished note; the caller checks whether its install is present.
pub fn read_unfinished(config_dir: &Path) -> Result<Option<Unfinished>, String> {
    let path = crate::unfinished_path(config_dir);
    read_optional(&path)?
        .map(|text| Unfinished::parse(&text, &path))
        .transpose()
}

/// Saves the setup record with the original write-then-rename operation.
pub fn write_record(config_dir: &Path, record: &Record) -> Result<(), String> {
    write_whole(&crate::record_path(config_dir), &record.render()?)
}

/// Saves the unfinished note with the same write-then-rename operation.
pub fn write_unfinished(config_dir: &Path, unfinished: &Unfinished) -> Result<(), String> {
    write_whole(&crate::unfinished_path(config_dir), &unfinished.render()?)
}

/// Removes the unfinished note, accepting an already-missing file.
pub fn remove_unfinished(config_dir: &Path) -> Result<(), String> {
    let path = crate::unfinished_path(config_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove {}: {e}", path.display())),
    }
}

/// Makes the default database's parent folder.
pub fn make_data_folder(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))
}

/// This program's absolute path, with links followed.
pub fn program_path() -> std::io::Result<PathBuf> {
    std::env::current_exe().and_then(|p| p.canonicalize())
}

/// This program's absolute path, for client configurations that run it.
pub fn kkt_path() -> Result<PathBuf, String> {
    program_path().map_err(|e| format!("could not find kkt's path: {e}"))
}

/// One snapshot of PATH, shared by all client checks in a setup invocation.
pub fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

/// Whether a program is a file in the already-captured PATH folders.
pub fn on_path(dirs: &[PathBuf], program: &str) -> bool {
    dirs.iter().any(|dir| dir.join(program).is_file())
}

/// Resolves a relative path against this process's working directory.
pub fn absolute(path: &Path) -> std::io::Result<PathBuf> {
    std::path::absolute(path)
}

/// Follows links and resolves the actual file path.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    path.canonicalize()
}

/// Writes whole, then renames into place, so a failure leaves the earlier file.
fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let partial = path.with_extension("toml.partial");
    std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Removes the setup record; missing files retain the caller's original refusal.
pub fn remove_record(config_dir: &Path) -> Result<(), String> {
    let path = crate::record_path(config_dir);
    std::fs::remove_file(&path).map_err(|e| format!("could not remove {}: {e}", path.display()))
}
