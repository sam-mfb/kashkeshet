//! Writes backup layouts and removes old backup folders selected by the
//! backup logic. Database copying is supplied by the program.

#![cfg(any(test, feature = "real"))]

use kkt_backup::{DATABASE_FILE, Layout};
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Writes under a private partial name, then renames when complete.
/// A failed fill removes the partial folder; the database copy runs first.
pub fn write(
    folder: &Path,
    layout: &Layout<'_>,
    copy_database: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    let done = folder.join(layout.name());
    if done.exists() {
        return Err(format!("{} already exists", done.display()));
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
        .map_err(|e| format!("could not create {}: {e}", partial.display()))?;
    match fill(&partial, layout, copy_database) {
        Ok(()) => std::fs::rename(&partial, &done)
            .map_err(|e| format!("could not rename {}: {e}", partial.display()))
            .map(|()| done),
        Err(e) => Err(match std::fs::remove_dir_all(&partial) {
            Ok(()) => e,
            Err(removing) => format!(
                "{e}; could not remove the unfinished {}: {removing}",
                partial.display()
            ),
        }),
    }
}

fn fill(
    into: &Path,
    layout: &Layout<'_>,
    copy_database: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    copy_database(layout.database(), &into.join(DATABASE_FILE)).map_err(|e| {
        format!(
            "could not copy the database {}: {e}",
            layout.database().display()
        )
    })?;
    for file in layout.copies() {
        let to = into.join(file.name);
        if file.private {
            copy_private(file.source, &to, layout.private_file_mode())?;
        } else {
            std::fs::copy(file.source, &to)
                .map_err(|e| format!("could not copy {}: {e}", file.source.display()))?;
        }
    }
    std::fs::write(into.join(layout.about_file()), layout.about()).map_err(|e| {
        format!(
            "could not write {}: {e}",
            into.join(layout.about_file()).display()
        )
    })
}

fn copy_private(from: &Path, to: &Path, mode: u32) -> Result<(), String> {
    let text =
        std::fs::read(from).map_err(|e| format!("could not read {}: {e}", from.display()))?;
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
        .map_err(|e| format!("could not write {}: {e}", to.display()))
}

/// Lists the folder, asks the backup logic which names are old, and
/// removes only those. A missing folder has no backups.
pub fn prune(folder: &Path, keep: usize) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("could not list {}: {e}", folder.display())),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not list {}: {e}", folder.display()))?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    let mut removed = Vec::new();
    for name in kkt_backup::old_names(names, keep) {
        let path = folder.join(name);
        std::fs::remove_dir_all(&path)
            .map_err(|e| format!("could not remove {}: {e}", path.display()))?;
        removed.push(path);
    }
    Ok(removed)
}

#[cfg(test)]
use kkt_scratch as scratch;
#[cfg(test)]
mod tests;
