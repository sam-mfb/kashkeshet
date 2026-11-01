//! Lanes as files on disk: a folder with one Markdown file per lane,
//! `<lane>.md` (see [`format`]), which `kkt lanes export` writes and `kkt
//! lanes import` reads, and the folder's git commit, which an import
//! records. Everything on disk goes through [`Disk`], so this part is tested
//! against a fake; [`OsDisk`] is the real one.
//!
//! This part only reads and writes files. Whether a lane is acceptable, and
//! what an import changes, is for the lane rules (`rules::Operator`).

mod format;
mod os;
#[cfg(test)]
mod tests;

pub use format::{EXTENSION, LaneFile, parse, render};
pub use os::OsDisk;

use std::io;
use std::path::{Path, PathBuf};

/// What this part needs from the machine.
pub trait Disk {
    /// The names of the files (not folders) directly in `folder`.
    fn file_names(&self, folder: &Path) -> io::Result<Vec<String>>;
    fn read(&self, path: &Path) -> io::Result<String>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    /// Writes `contents`, replacing any file there.
    fn write(&self, path: &Path, contents: &str) -> io::Result<()>;
    /// `path` made absolute, from the current folder.
    fn absolute(&self, path: &Path) -> io::Result<PathBuf>;
    /// The git commit checked out where `folder` is, with `-dirty` added
    /// when files in `folder` differ from it; `None` when `folder` is not in
    /// a git checkout (or one with no commit yet), git ignores it, or git is
    /// not installed.
    fn git_commit(&self, folder: &Path) -> io::Result<Option<String>>;
}

/// The lanes in a folder, and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// In file name order.
    pub lanes: Vec<LaneFile>,
    /// The folder, absolute.
    pub folder: PathBuf,
    pub commit: Option<String>,
}

/// Reads every `.md` file directly in `folder` as a lane; other files and
/// folders are left alone. A file that is not a lane file is an error
/// naming it, so a mistake is never skipped.
pub fn read_folder(disk: &dyn Disk, folder: &Path) -> Result<Read, String> {
    let absolute = disk
        .absolute(folder)
        .map_err(|e| format!("could not find {}: {e}", folder.display()))?;
    let mut names = disk
        .file_names(&absolute)
        .map_err(|e| format!("could not read the folder {}: {e}", absolute.display()))?;
    names.sort();
    let mut lanes = Vec::new();
    for name in names {
        let Some(stem) = name.strip_suffix(&format!(".{EXTENSION}")) else {
            continue;
        };
        let path = absolute.join(&name);
        let contents = disk
            .read(&path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        lanes.push(parse(stem, &contents).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    let commit = disk
        .git_commit(&absolute)
        .map_err(|e| format!("could not ask git about {}: {e}", absolute.display()))?;
    Ok(Read {
        lanes,
        folder: absolute,
        commit,
    })
}

/// Writes each lane to `folder` as `<lane>.md`, creating the folder if it
/// is missing and replacing a file of the same name. Other files are left
/// alone. Returns the absolute folder.
pub fn write_folder(disk: &dyn Disk, folder: &Path, lanes: &[LaneFile]) -> Result<PathBuf, String> {
    let absolute = disk
        .absolute(folder)
        .map_err(|e| format!("could not find {}: {e}", folder.display()))?;
    disk.create_dir_all(&absolute)
        .map_err(|e| format!("could not create {}: {e}", absolute.display()))?;
    for lane in lanes {
        let path = absolute.join(file_name(&lane.name));
        disk.write(&path, &render(lane))
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    Ok(absolute)
}

/// The file a lane is kept in.
pub fn file_name(lane: &str) -> String {
    format!("{lane}.{EXTENSION}")
}
