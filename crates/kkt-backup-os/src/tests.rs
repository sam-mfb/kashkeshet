use super::{CopyError, OsBackups};
use kkt_backup::{Backups, Contents, DATABASE_FILE, layout, name};
use kkt_clock::{Span, Time};
use std::path::{Path, PathBuf};

const CONFIG_FILE: &str = "config.toml";
const ACCESS_KEY_FILE: &str = "access-key";
const WEB_KEY_FILE: &str = "web-key";
const ADMIN_KEY_FILE: &str = "admin-key";
const ADMIN_NAME_FILE: &str = "admin-key.name";
const ABOUT_FILE: &str = "about.txt";
const PARTIAL: &str = ".partial";

// The existing scenarios exercise both the pure layout and the real writer.
fn write(
    folder: &Path,
    at: Time,
    version: &str,
    contents: &Contents<'_>,
    copy_database: impl Fn(&Path, &Path) -> Result<(), CopyError>,
) -> Result<PathBuf, String> {
    OsBackups::new(copy_database)
        .write(folder, &layout(at, version, contents))
        .map_err(|e| e.to_string())
}

/// Pruning, as the program's backups do it: the database copy is not used.
fn prune(folder: &Path, keep: usize) -> Result<Vec<PathBuf>, String> {
    OsBackups::new(|_: &Path, _: &Path| Err("not copied".into()))
        .prune(folder, keep)
        .map_err(|e| e.to_string())
}

fn at(seconds: u32) -> Time {
    Time::from_parts(2026, 10, 28, 13, 15, seconds).expect("time")
}

/// A fresh, empty directory for one test's files, removed when dropped.
fn scratch_dir(test: &str) -> kkt_scratch::Dir {
    kkt_scratch::Dir::new(&format!("kashkeshet-backup-{test}"))
}

/// A database "copy" that writes the source's text, as the store's would
/// write its pages.
fn copy_text(from: &Path, to: &Path) -> Result<(), CopyError> {
    std::fs::copy(from, to).map(|_| ()).map_err(CopyError::from)
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn a_backup_holds_each_file_and_keeps_the_key_private() {
    let dir = scratch_dir("write");
    let (database, config, key) = (dir.join("live.db"), dir.join("c.toml"), dir.join("key"));
    std::fs::write(&database, "pages").expect("db");
    std::fs::write(&config, "bind = \"127.0.0.1:7420\"").expect("config");
    std::fs::write(&key, "secret").expect("key");
    let contents = Contents {
        database: &database,
        config: Some(&config),
        access_key: Some(&key),
        web_key: Some(&key),
        admin_key: None,
        admin_name: None,
    };

    let folder = dir.join("backups");
    let written = write(&folder, at(0), "0.3.0", &contents, copy_text).expect("backup");

    assert_eq!(written, folder.join("kkt-20261028T131500.000Z-v0.3.0"));
    let read = |file: &str| std::fs::read_to_string(written.join(file)).expect(file);
    assert_eq!(read(DATABASE_FILE), "pages");
    assert_eq!(read(CONFIG_FILE), "bind = \"127.0.0.1:7420\"");
    assert_eq!(read(ACCESS_KEY_FILE), "secret");
    assert_eq!(read(WEB_KEY_FILE), "secret");
    assert_eq!(
        read(ABOUT_FILE),
        format!(
            "kkt 0.3.0 backup made 2026-10-28T13:15:00Z\n\
             kashkeshet.db: copy of {}\n\
             config.toml: copy of {}\n\
             access-key: copy of {}\n\
             web-key: copy of {}\n",
            database.display(),
            config.display(),
            key.display(),
            key.display()
        )
    );
    #[cfg(unix)]
    {
        assert_eq!(mode(&written), 0o700);
        assert_eq!(mode(&written.join(CONFIG_FILE)), 0o600);
        assert_eq!(mode(&written.join(ABOUT_FILE)), 0o600);
        assert_eq!(mode(&written.join(ACCESS_KEY_FILE)), 0o600);
        assert_eq!(mode(&written.join(WEB_KEY_FILE)), 0o600);
    }
    // The same moment again would overwrite it, so it is refused.
    assert!(write(&folder, at(0), "0.3.0", &contents, copy_text).is_err());
}

#[test]
fn the_about_file_says_the_time_to_the_second_and_the_folder_to_the_millisecond() {
    let dir = scratch_dir("about-time");
    let database = dir.join("live.db");
    std::fs::write(&database, "pages").expect("db");
    let contents = Contents {
        database: &database,
        config: None,
        access_key: None,
        web_key: None,
        admin_key: None,
        admin_name: None,
    };
    let at = at(7) + Span::milliseconds(250);

    let written = write(&dir.join("b"), at, "0.3.0", &contents, copy_text).expect("backup");

    assert_eq!(
        written.file_name().and_then(|n| n.to_str()),
        Some("kkt-20261028T131507.250Z-v0.3.0")
    );
    let about = std::fs::read_to_string(written.join(ABOUT_FILE)).expect("about");
    let first = about.lines().next().expect("first line");
    // Exactly the text the program gave `write` before it took only `at`.
    assert_eq!(
        first,
        format!("kkt 0.3.0 backup made {}", kkt_clock::to_text(at))
    );
    assert_eq!(first, "kkt 0.3.0 backup made 2026-10-28T13:15:07Z");
}

#[test]
fn an_admin_key_is_backed_up_privately_and_named_in_about() {
    let dir = scratch_dir("admin");
    let (database, key, admin) = (dir.join("live.db"), dir.join("key"), dir.join("admin"));
    std::fs::write(&database, "pages").expect("db");
    std::fs::write(&key, "secret").expect("key");
    std::fs::write(&admin, "admin-secret").expect("admin key");
    let contents = Contents {
        database: &database,
        config: None,
        access_key: Some(&key),
        web_key: Some(&key),
        admin_key: Some(&admin),
        admin_name: None,
    };
    let written = write(&dir.join("b"), at(0), "0.6.0", &contents, copy_text).expect("backup");
    assert_eq!(
        std::fs::read_to_string(written.join(ADMIN_KEY_FILE)).expect("admin key"),
        "admin-secret"
    );
    let about = std::fs::read_to_string(written.join(ABOUT_FILE)).expect("about");
    assert!(
        about.ends_with(&format!("admin-key: copy of {}\n", admin.display())),
        "{about}"
    );
    assert!(!about.contains("admin-secret"), "{about}");
    assert!(!written.join(ADMIN_NAME_FILE).exists());
    #[cfg(unix)]
    assert_eq!(mode(&written.join(ADMIN_KEY_FILE)), 0o600);
}

#[test]
fn an_admin_keys_name_is_backed_up_beside_it() {
    let dir = scratch_dir("admin-name");
    let (database, admin, name) = (dir.join("live.db"), dir.join("admin"), dir.join("name"));
    std::fs::write(&database, "pages").expect("db");
    std::fs::write(&admin, "admin-secret").expect("admin key");
    std::fs::write(&name, "admin-sentinel\n").expect("admin name");
    let contents = Contents {
        database: &database,
        config: None,
        access_key: None,
        web_key: None,
        admin_key: Some(&admin),
        admin_name: Some(&name),
    };
    let written = write(&dir.join("b"), at(0), "0.6.0", &contents, copy_text).expect("backup");
    assert_eq!(
        std::fs::read_to_string(written.join(ADMIN_NAME_FILE)).expect("admin name"),
        "admin-sentinel\n"
    );
    #[cfg(unix)]
    assert_eq!(mode(&written.join(ADMIN_NAME_FILE)), 0o600);
    let about = std::fs::read_to_string(written.join(ABOUT_FILE)).expect("about");
    assert!(
        about.ends_with(&format!("admin-key.name: copy of {}\n", name.display())),
        "{about}"
    );
}

#[test]
fn without_config_or_key_only_the_database_is_backed_up() {
    let dir = scratch_dir("db-only");
    let database = dir.join("live.db");
    std::fs::write(&database, "pages").expect("db");
    let contents = Contents {
        database: &database,
        config: None,
        access_key: None,
        web_key: None,
        admin_key: None,
        admin_name: None,
    };
    let written = write(&dir.join("b"), at(0), "0.3.0", &contents, copy_text).expect("backup");
    let mut files: Vec<_> = std::fs::read_dir(&written)
        .expect("list")
        .map(|e| e.expect("entry").file_name().into_string().expect("name"))
        .collect();
    files.sort();
    assert_eq!(files, [ABOUT_FILE, DATABASE_FILE]);
    let about = std::fs::read_to_string(written.join(ABOUT_FILE)).expect("about");
    let lines: Vec<&str> = about.lines().skip(1).collect();
    assert_eq!(
        lines,
        [
            format!("kashkeshet.db: copy of {}", database.display()).as_str(),
            "config.toml: none",
            "access-key: none",
            "web-key: none",
        ]
    );
}

#[test]
fn a_failed_backup_leaves_nothing_behind() {
    let dir = scratch_dir("fail");
    let database = dir.join("live.db");
    std::fs::write(&database, "pages").expect("db");
    let contents = Contents {
        database: &database,
        config: None,
        // Missing, so copying it fails after the database is copied.
        access_key: Some(&dir.join("no-such-key")),
        web_key: None,
        admin_key: None,
        admin_name: None,
    };
    let folder = dir.join("backups");
    let error = write(&folder, at(0), "0.3.0", &contents, copy_text).expect_err("fails");
    assert!(error.contains("no-such-key"), "{error}");
    assert_eq!(std::fs::read_dir(&folder).expect("list").count(), 0);

    let failing = |_: &Path, _: &Path| Err("disk full".into());
    let error = write(&folder, at(1), "0.3.0", &contents, failing).expect_err("fails");
    assert!(error.contains("disk full"), "{error}");
    assert_eq!(std::fs::read_dir(&folder).expect("list").count(), 0);
}

#[test]
fn pruning_keeps_the_newest_backups_and_nothing_else_is_touched() {
    let dir = scratch_dir("prune");
    let backups: Vec<String> = (0..5).map(|s| name(at(s), "0.3.0")).collect();
    for backup in &backups {
        std::fs::create_dir(dir.join(backup)).expect("backup");
    }
    std::fs::write(dir.join("notes.txt"), "mine").expect("other file");
    let partial = format!("{}{PARTIAL}", name(at(9), "0.3.0"));
    std::fs::create_dir(dir.join(&partial)).expect("partial");

    let removed = prune(&dir, 2).expect("prune");

    assert_eq!(
        removed,
        [
            dir.join(&backups[0]),
            dir.join(&backups[1]),
            dir.join(&backups[2])
        ]
    );
    let mut left: Vec<_> = std::fs::read_dir(&dir)
        .expect("list")
        .map(|e| e.expect("entry").file_name().into_string().expect("name"))
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            backups[3].clone(),
            backups[4].clone(),
            partial,
            "notes.txt".into()
        ]
    );
    assert!(
        prune(&dir.join("missing"), 2)
            .expect("missing folder")
            .is_empty()
    );
}

/// Every message a person can be shown, pinned exactly: kkt's words around
/// the operating system's own cause, which each case gets by doing the same
/// thing itself.
#[test]
fn every_backup_message_says_what_it_always_said() {
    let dir = scratch_dir("messages");
    let database = dir.join("live.db");
    std::fs::write(&database, "pages").expect("db");
    let bare = Contents {
        database: &database,
        config: None,
        access_key: None,
        web_key: None,
        admin_key: None,
        admin_name: None,
    };
    let backup = name(at(0), "0.3.0");

    // The folder cannot be made: a file stands where its parent should be.
    let file = dir.join("a-file");
    std::fs::write(&file, "").expect("file");
    let under_file = file.join("backups");
    let cause = std::fs::create_dir_all(&under_file).expect_err("not a folder");
    assert_eq!(
        write(&under_file, at(0), "0.3.0", &bare, copy_text).expect_err("fails"),
        format!("could not create {}: {cause}", under_file.display())
    );

    // A backup of that name is already there.
    let folder = dir.join("taken");
    std::fs::create_dir_all(folder.join(&backup)).expect("taken");
    assert_eq!(
        write(&folder, at(0), "0.3.0", &bare, copy_text).expect_err("fails"),
        format!("{} already exists", folder.join(&backup).display())
    );

    // The partial folder cannot be made: a file has its name.
    let folder = dir.join("partial");
    std::fs::create_dir_all(&folder).expect("folder");
    let partial = folder.join(format!("{backup}{PARTIAL}"));
    std::fs::write(&partial, "").expect("in the way");
    let cause = std::fs::create_dir(&partial).expect_err("in the way");
    assert_eq!(
        write(&folder, at(0), "0.3.0", &bare, copy_text).expect_err("fails"),
        format!("could not create {}: {cause}", partial.display())
    );

    // The database copy fails, in the program's words.
    let folder = dir.join("database");
    let failing = |_: &Path, _: &Path| Err("disk full".into());
    assert_eq!(
        write(&folder, at(0), "0.3.0", &bare, failing).expect_err("fails"),
        format!(
            "could not copy the database {}: disk full",
            database.display()
        )
    );

    // A config file cannot be read.
    let missing = dir.join("no-such-config");
    let cause = std::fs::copy(&missing, dir.join("copied")).expect_err("missing");
    let contents = Contents {
        config: Some(&missing),
        ..bare
    };
    assert_eq!(
        write(&dir.join("config"), at(0), "0.3.0", &contents, copy_text).expect_err("fails"),
        format!("could not read {}: {cause}", missing.display())
    );

    // A private file cannot be read.
    let missing = dir.join("no-such-key");
    let cause = std::fs::read(&missing).expect_err("missing");
    let contents = Contents {
        access_key: Some(&missing),
        ..bare
    };
    assert_eq!(
        write(&dir.join("key"), at(0), "0.3.0", &contents, copy_text).expect_err("fails"),
        format!("could not read {}: {cause}", missing.display())
    );

    // The backups' folder cannot be listed: it is a file.
    let cause = std::fs::read_dir(&file).expect_err("not a folder");
    assert_eq!(
        prune(&file, 1).expect_err("fails"),
        format!("could not list {}: {cause}", file.display())
    );
}
