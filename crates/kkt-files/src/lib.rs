//! General file access on this machine: reading and writing whole files,
//! listing a folder's file names, making folders, renaming and removing,
//! whether a path exists, creating a file only its owner can read and
//! restricting one to its owner, and absolute and canonical paths. It knows
//! nothing of whose files these are: each program adapts [`Files`] to the
//! contract the crate that needs files declares.

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

    /// Writes `text` as the file at `path`, whole, replacing any file there.
    pub fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        fs::write(path, text)
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

    /// Whether this process can see something at `path`. A path in a folder
    /// it cannot search does not exist, to this answer.
    pub fn exists(&self, path: &Path) -> bool {
        path.exists()
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
}
