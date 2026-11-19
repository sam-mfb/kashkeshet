//! Lanes as files on disk: a folder with one Markdown file per lane,
//! `<lane>.md` (see [`format`]), which `kkt lanes export` writes and `kkt
//! lanes import` reads, and the folder's git commit, which an import
//! records. A `README.md`, or a Markdown file with no lane header, is
//! skipped, so a folder can hold notes beside its lanes. Everything on disk
//! goes through [`Disk`], and the commit through [`LaneCommit`], so this
//! part is tested against fakes. The program supplies both through
//! adapters: the disk over kkt-files, the commit over
//! kkt-revision's git.
//!
//! It reads and writes the files. Whether a lane is acceptable, and what an
//! import changes, are the lane rules' (`rules::Operator`).

mod commit;
mod format;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

pub use commit::LaneCommit;
use format::EXTENSION;
pub use format::{HeaderKey, LaneFile, ParseError, parse, render};

use format::has_header;
use std::fmt;
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
}

/// Why a lane folder could not be read: the step that failed, with the
/// folder or file it was on.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("could not find {}: {source}", folder.display())]
    Find {
        folder: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read the folder {}: {source}", folder.display())]
    Folder {
        folder: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read {}: {source}", path.display())]
    File {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{}: {source}", path.display())]
    Lane {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
    #[error("could not ask git about {}: {source}", folder.display())]
    Commit {
        folder: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Why lanes could not be written to a folder: the step that failed, with
/// the folder or file it was on.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("could not find {}: {source}", folder.display())]
    Find {
        folder: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not create {}: {source}", folder.display())]
    Create {
        folder: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// The file every folder may hold to describe itself, never read as a lane.
const README: &str = "README.md";

/// The lanes in a folder, and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// In file name order.
    pub lanes: Vec<LaneFile>,
    /// The `.md` files not read as lanes, in file name order.
    pub skipped: Vec<Skipped>,
    /// The folder, absolute.
    pub folder: PathBuf,
    pub commit: Option<String>,
}

impl Read {
    /// The names, without `.md`, of the files skipped for having no lane
    /// header: each may be a lane's own file that lost its header, so an
    /// import must not retire those lanes. `README.md` is not one, as no
    /// lane can be named `README`.
    pub fn skipped_names(&self) -> Vec<String> {
        self.skipped
            .iter()
            .filter(|s| s.why == Skip::NoHeader)
            .filter_map(|s| s.path.file_stem()?.to_str().map(str::to_string))
            .collect()
    }

    /// The files an import elsewhere (kkt in Docker) needs copied to read
    /// this folder as this read did: the lane files, and with
    /// `retire_missing` the files skipped for having no header, so that
    /// import skips them too and does not retire their lanes.
    pub fn files_to_copy(&self, retire_missing: bool) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self
            .lanes
            .iter()
            .map(|l| self.folder.join(file_name(&l.name)))
            .collect();
        if retire_missing {
            files.extend(
                self.skipped_names()
                    .iter()
                    .map(|name| self.folder.join(file_name(name))),
            );
        }
        files
    }
}

/// A `.md` file in a lane folder that is not read as a lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// Absolute.
    pub path: PathBuf,
    pub why: Skip,
}

/// Why a `.md` file is not read as a lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// It is the folder's `README.md`.
    Readme,
    /// It does not start with a lane header.
    NoHeader,
}

/// The warning for a skipped file: `skipped <path>: <why>`.
impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match self.why {
            Skip::Readme => "a README is not a lane",
            Skip::NoHeader => "it does not start with a lane header (a --- line)",
        };
        write!(f, "skipped {}: {why}", self.path.display())
    }
}

/// Reads every `.md` file directly in `folder` as a lane; other files and
/// folders are left alone. `README.md`, and a `.md` file that does not
/// start with a lane header, are skipped and listed. A file with a header
/// that is otherwise wrong is an error naming it, so a mistake in a lane
/// is never skipped. The commit the folder is at comes from `commits`.
pub fn read_folder(
    disk: &dyn Disk,
    commits: &dyn LaneCommit,
    folder: &Path,
) -> Result<Read, ReadError> {
    let absolute = disk.absolute(folder).map_err(|source| ReadError::Find {
        folder: folder.to_path_buf(),
        source,
    })?;
    let mut names = disk
        .file_names(&absolute)
        .map_err(|source| ReadError::Folder {
            folder: absolute.clone(),
            source,
        })?;
    names.sort();
    let mut lanes = Vec::new();
    let mut skipped = Vec::new();
    for name in names {
        let Some(stem) = name.strip_suffix(&format!(".{EXTENSION}")) else {
            continue;
        };
        let path = absolute.join(&name);
        if name == README {
            skipped.push(Skipped {
                path,
                why: Skip::Readme,
            });
            continue;
        }
        let contents = disk.read(&path).map_err(|source| ReadError::File {
            path: path.clone(),
            source,
        })?;
        if !has_header(&contents) {
            skipped.push(Skipped {
                path,
                why: Skip::NoHeader,
            });
            continue;
        }
        lanes.push(parse(stem, &contents).map_err(|source| ReadError::Lane { path, source })?);
    }
    let commit = commits
        .commit(&absolute)
        .map_err(|source| ReadError::Commit {
            folder: absolute.clone(),
            source,
        })?;
    Ok(Read {
        lanes,
        skipped,
        folder: absolute,
        commit,
    })
}

/// Writes each lane to `folder` as `<lane>.md`, creating the folder if it
/// is missing and replacing a file of the same name. Other files are left
/// alone. Returns the absolute folder.
pub fn write_folder(
    disk: &dyn Disk,
    folder: &Path,
    lanes: &[LaneFile],
) -> Result<PathBuf, WriteError> {
    let absolute = disk.absolute(folder).map_err(|source| WriteError::Find {
        folder: folder.to_path_buf(),
        source,
    })?;
    disk.create_dir_all(&absolute)
        .map_err(|source| WriteError::Create {
            folder: absolute.clone(),
            source,
        })?;
    for lane in lanes {
        let path = absolute.join(file_name(&lane.name));
        disk.write(&path, &render(lane))
            .map_err(|source| WriteError::Write { path, source })?;
    }
    Ok(absolute)
}

/// The file a lane is kept in.
pub fn file_name(lane: &str) -> String {
    format!("{lane}.{EXTENSION}")
}
