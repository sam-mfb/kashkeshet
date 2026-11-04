use super::*;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Files in memory, under `/work`, with a git commit for some folders.
#[derive(Default)]
struct FakeDisk {
    files: RefCell<BTreeMap<PathBuf, String>>,
    folders: RefCell<Vec<PathBuf>>,
    commits: BTreeMap<PathBuf, String>,
}

impl FakeDisk {
    fn with(files: &[(&str, &str)]) -> FakeDisk {
        let disk = FakeDisk::default();
        for (path, contents) in files {
            let path = PathBuf::from(path);
            disk.folders
                .borrow_mut()
                .push(path.parent().expect("a folder").to_path_buf());
            disk.files.borrow_mut().insert(path, contents.to_string());
        }
        disk
    }
}

impl Disk for FakeDisk {
    fn file_names(&self, folder: &Path) -> io::Result<Vec<String>> {
        if !self.folders.borrow().iter().any(|f| f == folder) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such folder"));
        }
        // Newest first, to show the result does not depend on the order.
        Ok(self
            .files
            .borrow()
            .keys()
            .rev()
            .filter(|p| p.parent() == Some(folder))
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect())
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.folders.borrow_mut().push(path.to_path_buf());
        Ok(())
    }

    fn write(&self, path: &Path, contents: &str) -> io::Result<()> {
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), contents.to_string());
        Ok(())
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(Path::new("/work").join(path))
    }

    fn git_commit(&self, folder: &Path) -> io::Result<Option<String>> {
        Ok(self.commits.get(folder).cloned())
    }
}

fn lane(name: &str) -> LaneFile {
    LaneFile {
        name: name.into(),
        description: format!("The {name} lane."),
        text: format!("Be the {name}."),
    }
}

#[test]
fn a_folder_written_reads_back_with_its_commit() {
    let mut disk = FakeDisk::default();
    disk.commits
        .insert(PathBuf::from("/work/lanes"), "abc123".into());
    let lanes = [lane("reviewer"), lane("orchestrator")];
    let folder = write_folder(&disk, Path::new("lanes"), &lanes).expect("written");
    assert_eq!(folder, PathBuf::from("/work/lanes"));
    assert!(
        disk.files
            .borrow()
            .contains_key(Path::new("/work/lanes/reviewer.md"))
    );
    let read = read_folder(&disk, Path::new("lanes")).expect("read");
    assert_eq!(
        read,
        Read {
            lanes: vec![lane("orchestrator"), lane("reviewer")],
            skipped: Vec::new(),
            folder: PathBuf::from("/work/lanes"),
            commit: Some("abc123".into()),
        }
    );
}

#[test]
fn other_files_are_left_alone_and_a_folder_outside_git_has_no_commit() {
    let disk = FakeDisk::with(&[
        ("/work/lanes/notes.txt", "not a lane"),
        ("/work/lanes/reviewer.md", &render(&lane("reviewer"))),
    ]);
    let read = read_folder(&disk, Path::new("lanes")).expect("read");
    assert_eq!(read.lanes, vec![lane("reviewer")]);
    assert_eq!(read.skipped, []);
    assert_eq!(read.commit, None);
}

#[test]
fn a_readme_and_markdown_files_with_no_lane_header_are_skipped_and_named() {
    let disk = FakeDisk::with(&[
        // A README is skipped even when it starts like a lane.
        (
            "/work/lanes/README.md",
            "---\nname: README\ndescription: d\n---\n",
        ),
        ("/work/lanes/empty.md", ""),
        ("/work/lanes/notes.md", "\u{feff}# Notes\n\n---\n"),
        ("/work/lanes/reviewer.md", &render(&lane("reviewer"))),
    ]);
    let read = read_folder(&disk, Path::new("lanes")).expect("read");
    assert_eq!(read.lanes, vec![lane("reviewer")]);
    assert_eq!(
        read.skipped,
        vec![
            Skipped {
                path: PathBuf::from("/work/lanes/README.md"),
                why: Skip::Readme,
            },
            Skipped {
                path: PathBuf::from("/work/lanes/empty.md"),
                why: Skip::NoHeader,
            },
            Skipped {
                path: PathBuf::from("/work/lanes/notes.md"),
                why: Skip::NoHeader,
            },
        ]
    );
    // Only a file with no header may be a lane's file; README.md never is.
    assert_eq!(read.skipped_names(), ["empty", "notes"]);
    assert_eq!(
        read.skipped[0].to_string(),
        "skipped /work/lanes/README.md: a README is not a lane"
    );
    assert_eq!(
        read.skipped[2].to_string(),
        "skipped /work/lanes/notes.md: it does not start with a lane header (a --- line)"
    );
}

#[test]
fn the_files_to_copy_are_the_lane_files_and_with_retire_missing_the_headerless_ones() {
    let disk = FakeDisk::with(&[
        ("/work/lanes/README.md", "# My lanes"),
        ("/work/lanes/notes.md", "# Notes"),
        ("/work/lanes/reviewer.md", &render(&lane("reviewer"))),
    ]);
    let read = read_folder(&disk, Path::new("lanes")).expect("read");
    assert_eq!(
        read.files_to_copy(false),
        [PathBuf::from("/work/lanes/reviewer.md")]
    );
    assert_eq!(
        read.files_to_copy(true),
        [
            PathBuf::from("/work/lanes/reviewer.md"),
            PathBuf::from("/work/lanes/notes.md"),
        ]
    );
}

#[test]
fn a_folder_of_notes_reads_as_no_lanes_and_every_md_file_skipped() {
    // What the retire check (`rules::check_retire_missing`) is given.
    let notes_only = FakeDisk::with(&[
        ("/work/docs/README.md", "# Docs"),
        ("/work/docs/guide.md", "# Guide"),
    ]);
    let read = read_folder(&notes_only, Path::new("docs")).expect("read");
    assert!(read.lanes.is_empty());
    assert_eq!(read.skipped.len(), 2);
    assert_eq!(read.folder, PathBuf::from("/work/docs"));
    let with_lane = FakeDisk::with(&[
        ("/work/lanes/README.md", "# My lanes"),
        ("/work/lanes/reviewer.md", &render(&lane("reviewer"))),
    ]);
    let read = read_folder(&with_lane, Path::new("lanes")).expect("read");
    assert_eq!((read.lanes.len(), read.skipped.len()), (1, 1));
    let empty = FakeDisk::with(&[("/work/empty/notes.txt", "not a lane")]);
    let read = read_folder(&empty, Path::new("empty")).expect("read");
    assert_eq!((read.lanes.len(), read.skipped.len()), (0, 0));
}

#[test]
fn a_markdown_file_with_a_lane_header_that_is_wrong_is_an_error_naming_it() {
    let disk = FakeDisk::with(&[
        ("/work/lanes/notes.md", "# Notes"),
        ("/work/lanes/reviewer.md", "---\nname: reviewer\n"),
    ]);
    let error = read_folder(&disk, Path::new("lanes")).expect_err("refused");
    assert!(
        error.starts_with("/work/lanes/reviewer.md: the header has no closing ---"),
        "{error}"
    );
}

#[test]
fn a_missing_folder_is_an_error_naming_it() {
    let error = read_folder(&FakeDisk::default(), Path::new("gone")).expect_err("refused");
    assert!(error.contains("/work/gone"), "{error}");
}
