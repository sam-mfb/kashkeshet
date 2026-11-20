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
/// `copy_database(from, to)`, which creates its destination privately
/// (mode 0600 on Unix) and says why when it fails.
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

    fn finished(&self, folder: &Path) -> Result<Vec<PathBuf>, BackupError> {
        let mut finished = Vec::new();
        for path in backup_paths(folder, 0)? {
            let entry = std::fs::symlink_metadata(&path)
                .map_err(|e| BackupError::file(FileAction::List, folder, e))?;
            if entry.is_dir() {
                finished.push(path);
            }
        }
        Ok(finished)
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
        let mut source = std::fs::File::open(file.source)
            .map_err(|e| BackupError::file(FileAction::Read, file.source, e))?;
        let mut dest = private_file(&to, layout.private_file_mode())?;
        std::io::copy(&mut source, &mut dest)
            .map_err(|e| BackupError::file(FileAction::Copy, file.source, e))?;
    }
    let about = into.join(layout.about_file());
    private_file(&about, layout.private_file_mode())?
        .write_all(layout.about().as_bytes())
        .map_err(|e| BackupError::file(FileAction::Write, about, e))
}

fn private_file(to: &Path, mode: u32) -> Result<std::fs::File, BackupError> {
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
        .map_err(|e| BackupError::file(FileAction::Write, to, e))
}

/// Paths selected by the layout's naming and retention rules, in time order.
fn backup_paths(folder: &Path, keep: usize) -> Result<Vec<PathBuf>, BackupError> {
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
    Ok(kkt_backup::old_names(names, keep)
        .into_iter()
        .map(|name| folder.join(name))
        .collect())
}

fn prune(folder: &Path, keep: usize) -> Result<Vec<PathBuf>, BackupError> {
    let paths = backup_paths(folder, keep)?;
    for path in &paths {
        std::fs::remove_dir_all(path)
            .map_err(|e| BackupError::file(FileAction::Remove, path, e))?;
    }
    Ok(paths)
}

#[cfg(test)]
mod tests;
