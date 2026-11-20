//! General file access on this machine: reading and writing whole files
//! (a file that may be missing, and a write that replaces a file only once
//! the new text is whole), listing a folder's file names, making folders,
//! renaming and removing, whether a path exists or is a file, creating a file only its owner can read,
//! restricting one to its owner, telling and setting a file's permission
//! bits, and absolute and canonical paths. A caller
//! adapts [`Files`] to the file contract it needs.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The mode of a file only its owner can read and write.
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

/// The machine's files.
pub struct Files;

impl Files {
    /// The text of the file at `path`, whole.
    pub fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    /// The text of the file at `path`, whole; `None` if there is no file
    /// there.
    pub fn read_if_present(&self, path: &Path) -> io::Result<Option<String>> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes `text` as the file at `path`, whole, replacing any file there.
    pub fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        fs::write(path, text)
    }

    /// Writes `text` to `<path>.partial` beside the file, then renames it to
    /// `path`, so a failed write leaves any earlier file as it was.
    pub fn write_by_rename(&self, path: &Path, text: &str) -> io::Result<()> {
        let mut partial = path.as_os_str().to_owned();
        partial.push(".partial");
        let partial = PathBuf::from(partial);
        fs::write(&partial, text)?;
        fs::rename(&partial, path)
    }

    /// The names of the files in `folder`, in no set order; folders and
    /// other entries that are not files are left out. A name that is not
    /// UTF-8 is an error.
    pub fn file_names(&self, folder: &Path) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().into_string().map_err(|name| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the file name {name:?} is not UTF-8"),
                )
            })?;
            names.push(name);
        }
        Ok(names)
    }

    /// The entry names in `folder`, in no set order, as raw OS strings so an
    /// unrelated non-UTF-8 entry never fails the listing.
    pub fn dir_entries(&self, folder: &Path) -> io::Result<Vec<std::ffi::OsString>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(folder)? {
            names.push(entry?.file_name());
        }
        Ok(names)
    }

    /// Reads `candidate` as UTF-8 text if and only if it is a regular file
    /// (never a symbolic link) that is not the same underlying file as `other`
    /// (never a hard link to it). Returns `Ok(None)` when `candidate` is
    /// missing, is not a regular file, shares an inode with `other`, or when
    /// file independence cannot be established (such as on non-Unix systems).
    #[cfg(unix)]
    pub fn read_independent_file(
        &self,
        candidate: &Path,
        other: &Path,
    ) -> io::Result<Option<String>> {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        let meta = match fs::symlink_metadata(candidate) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if !meta.file_type().is_file() {
            return Ok(None);
        }
        let same_inode =
            |a: &fs::Metadata, b: &fs::Metadata| a.dev() == b.dev() && a.ino() == b.ino();
        let Ok(other_meta) = fs::metadata(other) else {
            return Ok(None);
        };
        let Ok(other_link) = fs::symlink_metadata(other) else {
            return Ok(None);
        };
        if same_inode(&meta, &other_meta) || same_inode(&meta, &other_link) {
            return Ok(None);
        }
        let mut file = match fs::File::open(candidate) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let open_meta = file.metadata()?;
        if !open_meta.file_type().is_file() || !same_inode(&meta, &open_meta) {
            return Ok(None);
        }
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        Ok(Some(text))
    }

    /// Reads `candidate` as UTF-8 text if and only if it is an independent
    /// regular file. On non-Unix systems where inode identity cannot be
    /// checked, declines reuse by returning `Ok(None)`.
    #[cfg(not(unix))]
    pub fn read_independent_file(
        &self,
        _candidate: &Path,
        _other: &Path,
    ) -> io::Result<Option<String>> {
        Ok(None)
    }

    /// Makes the folder at `path` and any folders above it that are missing.
    pub fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    /// Moves the file at `from` to `to`, replacing any file there.
    pub fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    /// Removes the file at `path`.
    pub fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    /// Removes the file at `path`; a file already missing is fine.
    pub fn remove_if_present(&self, path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    /// Whether this process can see something at `path`. A path in a folder
    /// it cannot search does not exist, to this answer.
    pub fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    /// Whether this process can see a file (not a folder) at `path`,
    /// following links.
    pub fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    /// Creates a new file at `path` holding `text`, which only its owner can
    /// read (on Unix), and makes sure it is on disk. Refuses a path where a
    /// file already is.
    pub fn create_private(&self, path: &Path, text: &str) -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(PRIVATE_FILE_MODE);
        }
        let mut file = options.open(path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()
    }

    /// The file's permission bits at `path`; `None` on a system that has
    /// none. An error when the file cannot be looked at.
    pub fn mode(&self, path: &Path) -> io::Result<Option<u32>> {
        let metadata = fs::metadata(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Ok(Some(metadata.permissions().mode() & 0o777))
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Ok(None)
        }
    }

    /// Sets the file's permission bits at `path` (on Unix; elsewhere it
    /// does nothing).
    pub fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = (path, mode);
        Ok(())
    }

    /// Lets only the file's owner read and write the file at `path` (on
    /// Unix; elsewhere it does nothing).
    pub fn restrict(&self, path: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
        }
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }

    /// `path` made absolute against the working folder, without touching
    /// the file system.
    pub fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        std::path::absolute(path)
    }

    /// `path` absolute and free of symbolic links and `.` and `..`; the path
    /// must exist.
    pub fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        path.canonicalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_scratch as scratch;

    #[test]
    fn a_written_file_reads_back_and_is_replaced_whole() {
        let dir = scratch::Dir::new("files-write");
        let path = dir.join("a.txt");
        Files.write(&path, "first text").expect("written");
        assert_eq!(Files.read(&path).expect("read"), "first text");
        Files.write(&path, "").expect("replaced");
        assert_eq!(Files.read(&path).expect("read"), "");
    }

    #[test]
    fn a_missing_file_cannot_be_read_and_does_not_exist() {
        let dir = scratch::Dir::new("files-missing");
        let path = dir.join("missing.txt");
        assert!(!Files.exists(&path));
        let err = Files.read(&path).expect_err("missing");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_file_that_may_be_missing_reads_as_none_when_it_is() {
        let dir = scratch::Dir::new("files-if-present");
        let path = dir.join("record.toml");
        assert_eq!(Files.read_if_present(&path).expect("no error"), None);
        Files.write(&path, "text").expect("written");
        assert_eq!(
            Files.read_if_present(&path).expect("read").as_deref(),
            Some("text")
        );
        // Any other failure is still an error: here, a folder in its place.
        let folder = dir.join("folder");
        Files.create_dir_all(&folder).expect("made");
        assert!(Files.read_if_present(&folder).is_err());
    }

    #[test]
    fn a_write_by_rename_replaces_the_file_and_leaves_no_partial_copy() {
        let dir = scratch::Dir::new("files-by-rename");
        let path = dir.join("setup.toml");
        Files.write(&path, "old").expect("written");
        Files.write_by_rename(&path, "new").expect("replaced");
        assert_eq!(Files.read(&path).expect("read"), "new");
        assert_eq!(Files.file_names(&dir).expect("listed"), ["setup.toml"]);
    }

    #[test]
    fn a_failed_write_by_rename_leaves_the_earlier_file() {
        let dir = scratch::Dir::new("files-by-rename-fails");
        let path = dir.join("setup.toml");
        Files.write(&path, "old").expect("written");
        // A folder where the partial copy goes makes the write fail.
        Files
            .create_dir_all(&dir.join("setup.toml.partial"))
            .expect("made");
        assert!(Files.write_by_rename(&path, "new").is_err());
        assert_eq!(Files.read(&path).expect("read"), "old");
    }

    #[test]
    fn remove_if_present_accepts_a_missing_file() {
        let dir = scratch::Dir::new("files-remove-if-present");
        let path = dir.join("note.toml");
        Files.remove_if_present(&path).expect("missing is fine");
        Files.write(&path, "x").expect("written");
        Files.remove_if_present(&path).expect("removed");
        assert!(!Files.exists(&path));
        // Any other failure is still an error: here, a folder in its place.
        Files.create_dir_all(&path).expect("made");
        assert!(Files.remove_if_present(&path).is_err());
    }

    #[test]
    fn is_file_is_true_for_files_only() {
        let dir = scratch::Dir::new("files-is-file");
        let file = dir.join("kkt");
        Files.write(&file, "").expect("written");
        assert!(Files.is_file(&file));
        assert!(!Files.is_file(&dir));
        assert!(!Files.is_file(&dir.join("missing")));
    }

    #[test]
    fn writing_into_a_missing_folder_fails_until_it_is_made() {
        let dir = scratch::Dir::new("files-folders");
        let path = dir.join("new").join("deeper").join("a.txt");
        assert!(Files.write(&path, "x").is_err());
        Files
            .create_dir_all(path.parent().expect("parent"))
            .expect("made");
        Files.write(&path, "x").expect("written");
        assert!(Files.exists(&path));
    }

    #[test]
    fn a_folder_cannot_be_made_under_a_file() {
        let dir = scratch::Dir::new("files-folder-under-file");
        let blocker = dir.join("file");
        Files.write(&blocker, "x").expect("written");
        assert!(Files.create_dir_all(&blocker.join("sub")).is_err());
    }

    #[test]
    fn file_names_lists_files_only() {
        let dir = scratch::Dir::new("files-names");
        Files.write(&dir.join("b.md"), "").expect("written");
        Files.write(&dir.join("a.md"), "").expect("written");
        Files.create_dir_all(&dir.join("folder")).expect("made");
        let mut names = Files.file_names(&dir).expect("listed");
        names.sort();
        assert_eq!(names, ["a.md", "b.md"]);
        assert!(Files.file_names(&dir.join("missing")).is_err());
    }

    #[test]
    fn rename_moves_a_file_over_another_and_remove_deletes_it() {
        let dir = scratch::Dir::new("files-rename");
        let from = dir.join("from");
        let to = dir.join("to");
        Files.write(&from, "new").expect("written");
        Files.write(&to, "old").expect("written");
        Files.rename(&from, &to).expect("renamed");
        assert!(!Files.exists(&from));
        assert_eq!(Files.read(&to).expect("read"), "new");
        Files.remove_file(&to).expect("removed");
        assert!(!Files.exists(&to));
        assert!(Files.remove_file(&to).is_err());
    }

    #[test]
    fn create_private_refuses_an_existing_file() {
        let dir = scratch::Dir::new("files-private");
        let path = dir.join("secret");
        Files.create_private(&path, "key").expect("created");
        assert_eq!(Files.read(&path).expect("read"), "key");
        let err = Files.create_private(&path, "other").expect_err("exists");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(Files.read(&path).expect("read"), "key");
    }

    #[test]
    #[cfg(unix)]
    fn private_files_and_restricted_files_are_for_their_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch::Dir::new("files-modes");
        let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
        let private = dir.join("private");
        Files.create_private(&private, "key").expect("created");
        assert_eq!(mode(&private), 0o600);
        let open = dir.join("open");
        Files.write(&open, "x").expect("written");
        fs::set_permissions(&open, fs::Permissions::from_mode(0o644)).expect("opened");
        Files.restrict(&open).expect("restricted");
        assert_eq!(mode(&open), 0o600);
    }

    #[test]
    #[cfg(unix)]
    fn a_files_mode_is_told_and_set() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch::Dir::new("files-mode-set");
        let path = dir.join("open");
        Files.write(&path, "x").expect("written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("opened");
        assert_eq!(Files.mode(&path).expect("told"), Some(0o644));
        Files.set_mode(&path, 0o640).expect("set");
        assert_eq!(Files.mode(&path).expect("told again"), Some(0o640));
        assert!(Files.mode(&dir.join("missing")).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn a_file_in_a_folder_that_cannot_be_searched_does_not_exist() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch::Dir::new("files-locked");
        let locked = dir.join("locked");
        let path = locked.join("config.toml");
        Files.create_dir_all(&locked).expect("made");
        Files.write(&path, "bind = 1\n").expect("written");
        assert!(Files.exists(&path));
        let set = |m| fs::set_permissions(&locked, fs::Permissions::from_mode(m));
        set(0o000).expect("locked");
        // Root searches any folder; the check means something only when the
        // lock holds.
        let holds = fs::metadata(&path).is_err();
        let exists = Files.exists(&path);
        let read = Files.read(&path);
        set(0o755).expect("unlocked");
        if holds {
            assert!(!exists);
            assert!(read.is_err(), "read reports what it cannot see");
        }
    }

    #[test]
    fn absolute_and_canonical_paths() {
        let dir = scratch::Dir::new("files-paths");
        let relative = Path::new("some").join("file");
        let absolute = Files.absolute(&relative).expect("absolute");
        assert!(absolute.is_absolute());
        assert!(absolute.ends_with(&relative));
        Files.create_dir_all(&dir.join("a")).expect("made");
        let roundabout = dir.join("a").join("..").join("a");
        assert_eq!(
            Files.canonicalize(&roundabout).expect("canonical"),
            dir.join("a").canonicalize().expect("canonical")
        );
        assert!(Files.canonicalize(&dir.join("missing")).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn read_independent_file_declines_when_other_metadata_cannot_be_read() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch::Dir::new("files-independent-unreadable");
        let locked = dir.join("locked");
        let other = locked.join("config.toml");
        let candidate = dir.join("config.toml.bak");
        Files.create_dir_all(&locked).expect("made");
        Files.write(&other, "original\n").expect("written");
        fs::hard_link(&other, &candidate).expect("hard link");

        // Accessible hard link is rejected.
        assert_eq!(
            Files
                .read_independent_file(&candidate, &other)
                .expect("checked"),
            None
        );

        // Missing `other` cannot be verified as independent, so reuse is declined.
        assert_eq!(
            Files
                .read_independent_file(&candidate, &dir.join("missing"))
                .expect("missing other"),
            None
        );

        // Inaccessible `other` (PermissionDenied) must also decline reuse.
        let set = |m| fs::set_permissions(&locked, fs::Permissions::from_mode(m));
        set(0o000).expect("locked");
        let holds = fs::metadata(&other).is_err();
        let result = Files.read_independent_file(&candidate, &other);
        set(0o755).expect("unlocked");
        if holds {
            assert_eq!(result.expect("declined"), None);
        }
    }

    #[test]
    #[cfg(not(unix))]
    fn read_independent_file_declines_reuse_on_non_unix() {
        let dir = scratch::Dir::new("files-independent-non-unix");
        let other = dir.join("config.toml");
        let candidate = dir.join("config.toml.bak");
        Files.write(&other, "original\n").expect("written");
        Files.write(&candidate, "original\n").expect("written");
        assert_eq!(
            Files
                .read_independent_file(&candidate, &other)
                .expect("declined"),
            None
        );
    }
}
