//! Backup layouts, names, file modes and pruning choices, with no file I/O.
//! The caller supplies the time and source paths. Behind `real`, `write`
//! carries out a layout on disk and `prune` removes the old folders
//! `old_names` chooses; only programs turn `real` on.

#[cfg(any(feature = "real", test))]
mod files;
mod settings;

#[cfg(feature = "real")]
pub use files::{prune, write};
pub use settings::{Problem, Settings};

use kkt_clock::Time;
use std::path::Path;

/// The database's name within a backup folder.
pub const DATABASE_FILE: &str = "kashkeshet.db";
const CONFIG_FILE: &str = "config.toml";
const ACCESS_KEY_FILE: &str = "access-key";
const WEB_KEY_FILE: &str = "web-key";
const ADMIN_KEY_FILE: &str = "admin-key";
const ADMIN_NAME_FILE: &str = "admin-key.name";
const ABOUT_FILE: &str = "about.txt";
const PREFIX: &str = "kkt-";
const PARTIAL: &str = ".partial";
const FOLDER_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;

/// The database and optional config and key files to include.
pub struct Contents<'a> {
    pub database: &'a Path,
    pub config: Option<&'a Path>,
    pub access_key: Option<&'a Path>,
    pub web_key: Option<&'a Path>,
    pub admin_key: Option<&'a Path>,
    /// The name tied to the admin key, if it has one.
    pub admin_name: Option<&'a Path>,
}

/// A backup's name, sorting by time and recording its writer's version.
pub fn name(at: Time, version: &str) -> String {
    format!("{PREFIX}{}-v{version}", at.to_compact_millis_text())
}

fn is_backup(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(PREFIX) else {
        return false;
    };
    let Some((time, version)) = rest.split_once("-v") else {
        return false;
    };
    !version.is_empty() && !name.ends_with(PARTIAL) && Time::is_compact_millis_text(time)
}

/// One source file and its name and privacy within a backup.
pub struct FileCopy<'a> {
    pub source: &'a Path,
    pub name: &'static str,
    pub private: bool,
}

/// An immutable backup layout. Only `layout` constructs it, so its modes
/// and filenames always follow the backup rules.
pub struct Layout<'a> {
    name: String,
    partial_name: String,
    database: &'a Path,
    copies: Vec<FileCopy<'a>>,
    about: String,
}

impl<'a> Layout<'a> {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn partial_name(&self) -> &str {
        &self.partial_name
    }
    pub fn database(&self) -> &Path {
        self.database
    }
    pub fn copies(&self) -> &[FileCopy<'a>] {
        &self.copies
    }
    pub fn about(&self) -> &str {
        &self.about
    }
    pub fn about_file(&self) -> &'static str {
        ABOUT_FILE
    }
    pub fn folder_mode(&self) -> u32 {
        FOLDER_MODE
    }
    pub fn private_file_mode(&self) -> u32 {
        PRIVATE_FILE_MODE
    }
}

/// The names, copies, metadata and privacy for a backup, without reading
/// or writing its files. The database is copied before these other files.
pub fn layout<'a>(at: Time, version: &str, contents: &Contents<'a>) -> Layout<'a> {
    let name = name(at, version);
    let copies = [
        (contents.config, CONFIG_FILE, false),
        (contents.access_key, ACCESS_KEY_FILE, true),
        (contents.web_key, WEB_KEY_FILE, true),
        (contents.admin_key, ADMIN_KEY_FILE, true),
        (contents.admin_name, ADMIN_NAME_FILE, false),
    ]
    .into_iter()
    .filter_map(|(source, name, private)| {
        source.map(|source| FileCopy {
            source,
            name,
            private,
        })
    })
    .collect();
    Layout {
        partial_name: format!("{name}{PARTIAL}"),
        name,
        database: contents.database,
        copies,
        about: about(at, version, contents),
    }
}

fn about(at: Time, version: &str, contents: &Contents<'_>) -> String {
    let source = |path: Option<&Path>| match path {
        Some(path) => format!("copy of {}", path.display()),
        None => "none".to_string(),
    };
    let at_text = at.to_text();
    let mut about = format!(
        "kkt {version} backup made {at_text}\n\
         {DATABASE_FILE}: {}\n\
         {CONFIG_FILE}: {}\n\
         {ACCESS_KEY_FILE}: {}\n\
         {WEB_KEY_FILE}: {}\n",
        source(Some(contents.database)),
        source(contents.config),
        source(contents.access_key),
        source(contents.web_key),
    );
    // Named only when there is one, so backups without it read as before.
    if contents.admin_key.is_some() {
        about.push_str(&format!(
            "{ADMIN_KEY_FILE}: {}\n",
            source(contents.admin_key)
        ));
    }
    if contents.admin_name.is_some() {
        about.push_str(&format!(
            "{ADMIN_NAME_FILE}: {}\n",
            source(contents.admin_name)
        ));
    }
    about
}

/// Finished backup names to remove, oldest first. Other names are ignored.
pub fn old_names(mut names: Vec<String>, keep: usize) -> Vec<String> {
    names.retain(|name| is_backup(name));
    names.sort();
    names.truncate(names.len().saturating_sub(keep));
    names
}

#[cfg(test)]
mod tests;
