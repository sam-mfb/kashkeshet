use super::*;
use crate::testing::Memory;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Files in memory, under `/work`.
#[derive(Default)]
struct FakeDisk {
    files: RefCell<BTreeMap<PathBuf, String>>,
    folders: RefCell<Vec<PathBuf>>,
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
    let disk = FakeDisk::default();
    let commits = Memory::new().at("/work/lanes", "abc123");
    let lanes = [lane("reviewer"), lane("orchestrator")];
    let folder = write_folder(&disk, Path::new("lanes"), &lanes).expect("written");
    assert_eq!(folder, PathBuf::from("/work/lanes"));
    assert!(
        disk.files
            .borrow()
            .contains_key(Path::new("/work/lanes/reviewer.md"))
    );
    let read = read_folder(&disk, &commits, Path::new("lanes")).expect("read");
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
    let read = read_folder(&disk, &Memory::new(), Path::new("lanes")).expect("read");
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
    let read = read_folder(&disk, &Memory::new(), Path::new("lanes")).expect("read");
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
    let read = read_folder(&disk, &Memory::new(), Path::new("lanes")).expect("read");
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
    let read = read_folder(&notes_only, &Memory::new(), Path::new("docs")).expect("read");
    assert!(read.lanes.is_empty());
    assert_eq!(read.skipped.len(), 2);
    assert_eq!(read.folder, PathBuf::from("/work/docs"));
    let with_lane = FakeDisk::with(&[
        ("/work/lanes/README.md", "# My lanes"),
        ("/work/lanes/reviewer.md", &render(&lane("reviewer"))),
    ]);
    let read = read_folder(&with_lane, &Memory::new(), Path::new("lanes")).expect("read");
    assert_eq!((read.lanes.len(), read.skipped.len()), (1, 1));
    let empty = FakeDisk::with(&[("/work/empty/notes.txt", "not a lane")]);
    let read = read_folder(&empty, &Memory::new(), Path::new("empty")).expect("read");
    assert_eq!((read.lanes.len(), read.skipped.len()), (0, 0));
}

#[test]
fn a_markdown_file_with_a_lane_header_that_is_wrong_is_an_error_naming_it() {
    let disk = FakeDisk::with(&[
        ("/work/lanes/notes.md", "# Notes"),
        ("/work/lanes/reviewer.md", "---\nname: reviewer\n"),
    ]);
    let error = read_folder(&disk, &Memory::new(), Path::new("lanes")).expect_err("refused");
    assert!(
        error.starts_with("/work/lanes/reviewer.md: the header has no closing ---"),
        "{error}"
    );
}

#[test]
fn a_missing_folder_is_an_error_naming_it() {
    let error =
        read_folder(&FakeDisk::default(), &Memory::new(), Path::new("gone")).expect_err("refused");
    assert!(error.contains("/work/gone"), "{error}");
}

/// A disk on which one step fails, with "refused", and every other step
/// works as `FakeDisk` does.
struct FailingDisk {
    fails: &'static str,
    disk: FakeDisk,
}

impl FailingDisk {
    fn refused(&self, step: &str) -> io::Result<()> {
        if self.fails == step {
            return Err(io::Error::other("refused"));
        }
        Ok(())
    }
}

impl Disk for FailingDisk {
    fn file_names(&self, folder: &Path) -> io::Result<Vec<String>> {
        self.refused("file_names")?;
        self.disk.file_names(folder)
    }
    fn read(&self, path: &Path) -> io::Result<String> {
        self.refused("read")?;
        self.disk.read(path)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.refused("create_dir_all")?;
        self.disk.create_dir_all(path)
    }
    fn write(&self, path: &Path, contents: &str) -> io::Result<()> {
        self.refused("write")?;
        self.disk.write(path, contents)
    }
    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        self.refused("absolute")?;
        self.disk.absolute(path)
    }
}

/// Git that cannot be asked.
struct NoGit;

impl LaneCommit for NoGit {
    fn commit(&self, _: &Path) -> io::Result<Option<String>> {
        Err(io::Error::other("refused"))
    }
}

/// Every refusal of a lane file, and of reading or writing a folder, word
/// for word.
#[test]
fn every_refusal_is_pinned() {
    let lane_file = |contents: &str| parse("reviewer", contents).map_err(|e| e.to_string());
    for (contents, refusal) in [
        (
            "# no header",
            "it must start with a --- line, then name: and description: lines, then ---",
        ),
        (
            "---\nname reviewer\n---\n",
            "header line \"name reviewer\" is not `name: ...` or `description: ...`",
        ),
        (
            "---\nowner: me\n---\n",
            "the header has \"owner\"; it takes only name and description",
        ),
        (
            "---\nname: reviewer\nname: reviewer\n---\n",
            "the header gives name twice",
        ),
        (
            "---\ndescription: d\ndescription: d\n---\n",
            "the header gives description twice",
        ),
        (
            "---\nname: reviewer\n",
            "the header has no closing --- line",
        ),
        ("---\ndescription: d\n---\n", "the header has no name: line"),
        (
            "---\nname: other\ndescription: d\n---\n",
            "the header names the lane \"other\", but the file is reviewer.md; they must match",
        ),
        (
            "---\nname: reviewer\n---\n",
            "the header has no description: line",
        ),
    ] {
        assert_eq!(
            lane_file(contents),
            Err(refusal.to_string()),
            "{contents:?}"
        );
    }

    let lanes = || {
        FakeDisk::with(&[
            ("/work/lanes/reviewer.md", "---\nname: reviewer\n"),
            ("/work/lanes/notes.txt", ""),
        ])
    };
    let read = |fails: &'static str, commits: &dyn LaneCommit| {
        read_folder(
            &FailingDisk {
                fails,
                disk: lanes(),
            },
            commits,
            Path::new("lanes"),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    };
    assert_eq!(
        read("absolute", &Memory::new()),
        Err("could not find lanes: refused".to_string())
    );
    assert_eq!(
        read("file_names", &Memory::new()),
        Err("could not read the folder /work/lanes: refused".to_string())
    );
    assert_eq!(
        read("read", &Memory::new()),
        Err("could not read /work/lanes/reviewer.md: refused".to_string())
    );
    assert_eq!(
        read("", &Memory::new()),
        Err("/work/lanes/reviewer.md: the header has no closing --- line".to_string())
    );
    let notes = FakeDisk::with(&[("/work/lanes/notes.md", "# Notes")]);
    assert_eq!(
        read_folder(&notes, &NoGit, Path::new("lanes"))
            .map(|_| ())
            .map_err(|e| e.to_string()),
        Err("could not ask git about /work/lanes: refused".to_string())
    );

    let write = |fails: &'static str| {
        write_folder(
            &FailingDisk {
                fails,
                disk: FakeDisk::default(),
            },
            Path::new("out"),
            &[lane("reviewer")],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    };
    assert_eq!(
        write("absolute"),
        Err("could not find out: refused".to_string())
    );
    assert_eq!(
        write("create_dir_all"),
        Err("could not create /work/out: refused".to_string())
    );
    assert_eq!(
        write("write"),
        Err("could not write /work/out/reviewer.md: refused".to_string())
    );
}
