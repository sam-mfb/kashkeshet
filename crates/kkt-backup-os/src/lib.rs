//! Backups on disk: [`OsBackups`] implements kkt-backup's [`Backups`]
//! contract with the operating system's files, writing kkt-backup's layouts
//! and removing the old backup folders kkt-backup picks. It is given the
//! database copy, which is the program's.

use kkt_backup::{ABOUT_FILE, BackupError, Backups, DATABASE_FILE, FileAction, Layout};
use std::error::Error;
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Backups in folders on this machine's disk, the database copied by
/// `copy_database(from, to)`, which says why when it fails.
pub struct OsBackups<C> {
    copy_database: C,
}

impl<C> OsBackups<C>
where
    C: Fn(&Path, &Path) -> Result<(), CopyError>,
{
    pub const fn new(copy_database: C) -> OsBackups<C> {
        OsBackups { copy_database }
    }
}

impl<C> Backups for OsBackups<C>
where
    C: Fn(&Path, &Path) -> Result<(), CopyError>,
{
    fn write(&self, folder: &Path, layout: &Layout<'_>) -> Result<PathBuf, BackupError> {
        write(folder, layout, &self.copy_database)
    }

    fn record_highest(&self, backup: &Path, highest: i64) -> Result<(), BackupError> {
        let about = backup.join(ABOUT_FILE);
        let mut file = OpenOptions::new()
            .append(true)
            .open(&about)
            .map_err(|e| BackupError::file(FileAction::Write, &about, e))?;
        file.write_all(kkt_backup::highest_line(highest).as_bytes())
            .map_err(|e| BackupError::file(FileAction::Write, &about, e))
    }

    fn prune(&self, folder: &Path, keep: usize) -> Result<Vec<PathBuf>, BackupError> {
        prune(folder, keep)
    }
}

/// Why the database copy failed.
pub type CopyError = Box<dyn Error + Send + Sync>;

/// Writes under a private partial name, then renames when complete.
/// A failed fill removes the partial folder; the database copy runs first.
fn write(
    folder: &Path,
    layout: &Layout<'_>,
    copy_database: impl FnOnce(&Path, &Path) -> Result<(), CopyError>,
) -> Result<PathBuf, BackupError> {
    std::fs::create_dir_all(folder)
        .map_err(|e| BackupError::file(FileAction::Create, folder, e))?;
    let done = folder.join(layout.name());
    if done.exists() {
        return Err(BackupError::Exists { path: done });
    }
    let partial = folder.join(layout.partial_name());
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(layout.folder_mode());
    }
    builder
        .create(&partial)
        .map_err(|e| BackupError::file(FileAction::Create, &partial, e))?;
    match fill(&partial, layout, copy_database) {
        Ok(()) => std::fs::rename(&partial, &done)
            .map_err(|e| BackupError::file(FileAction::Rename, &partial, e))
            .map(|()| done),
        Err(e) => Err(match std::fs::remove_dir_all(&partial) {
            Ok(()) => e,
            Err(removing) => BackupError::NotRemoved {
                error: Box::new(e),
                partial,
                cause: removing,
            },
        }),
    }
}

fn fill(
    into: &Path,
    layout: &Layout<'_>,
    copy_database: impl FnOnce(&Path, &Path) -> Result<(), CopyError>,
) -> Result<(), BackupError> {
    copy_database(layout.database(), &into.join(DATABASE_FILE)).map_err(|cause| {
        BackupError::Database {
            path: layout.database().to_path_buf(),
            cause,
        }
    })?;
    for file in layout.copies() {
        let to = into.join(file.name);
        if file.private {
            copy_private(file.source, &to, layout.private_file_mode())?;
        } else {
            std::fs::copy(file.source, &to)
                .map_err(|e| BackupError::file(FileAction::Copy, file.source, e))?;
        }
    }
    let about = into.join(layout.about_file());
    std::fs::write(&about, layout.about())
        .map_err(|e| BackupError::file(FileAction::Write, about, e))
}

fn copy_private(from: &Path, to: &Path, mode: u32) -> Result<(), BackupError> {
    let text = std::fs::read(from).map_err(|e| BackupError::file(FileAction::Read, from, e))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    options
        .open(to)
        .and_then(|mut file| file.write_all(&text))
        .map_err(|e| BackupError::file(FileAction::Write, to, e))
}

/// Lists the folder, asks the backup logic which names are old, and
/// removes only those. A missing folder has no backups.
fn prune(folder: &Path, keep: usize) -> Result<Vec<PathBuf>, BackupError> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(BackupError::file(FileAction::List, folder, e)),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| BackupError::file(FileAction::List, folder, e))?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    let mut removed = Vec::new();
    for name in kkt_backup::old_names(names, keep) {
        let path = folder.join(name);
        std::fs::remove_dir_all(&path)
            .map_err(|e| BackupError::file(FileAction::Remove, &path, e))?;
        removed.push(path);
    }
    Ok(removed)
}

#[cfg(test)]
mod tests;
