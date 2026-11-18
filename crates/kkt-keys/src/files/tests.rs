//! The reads and writes [`KeyFiles`] provides, over the in-memory stand-in.

use super::*;
use crate::testing::Memory;
use crate::{ACCESS_KEY_FILE_MODE, AccessKey};

/// The error of a read expected to fail. `KeyText` has no `Debug`, so that
/// a key is never printed, so `expect_err` cannot be used.
fn refused<T>(read: Result<T, ReadError>) -> ReadError {
    match read {
        Ok(_) => panic!("the read should have failed"),
        Err(e) => e,
    }
}

fn keys<'a>(access: &'a str, web: &'a str, admin: &'a str) -> SelectedKeys<'a> {
    SelectedKeys {
        access: Some(Path::new(access)),
        web: Some(Path::new(web)),
        admin: Some(Path::new(admin)),
    }
}

#[test]
fn selected_keys_are_read_with_where_they_came_from() {
    let files = Memory::new()
        .with_file("/k/access-key", "a\n")
        .with_file("/k/web-key", "w\n")
        .with_file("/k/admin-key", "d\n")
        .with_file("/k/admin-key.name", "n\n");
    let keys = keys("/k/access-key", "/k/web-key", "/k/admin-key");
    let access = files.read_access(&keys).expect("read").expect("access");
    assert_eq!(
        (access.text.as_str(), access.source.as_str()),
        ("a\n", "/k/access-key")
    );
    assert_eq!(
        files.read_web(&keys).expect("read").expect("web").text,
        "w\n"
    );
    assert_eq!(
        files.read_admin(&keys).expect("read").expect("admin").text,
        "d\n"
    );
    assert_eq!(
        files.read_admin_names(&keys).expect("read").as_deref(),
        Some("n\n")
    );
}

#[test]
fn keys_turned_off_are_not_read() {
    let files = Memory::new().failing("/k/access-key");
    let off = SelectedKeys::default();
    assert!(files.read_access(&off).expect("off").is_none());
    assert!(files.read_web(&off).expect("off").is_none());
    assert!(files.read_admin(&off).expect("off").is_none());
    assert!(files.read_admin_names(&off).expect("off").is_none());
}

#[test]
fn a_missing_access_or_web_key_is_an_error_naming_its_file() {
    let files = Memory::new();
    let keys = keys("/k/access-key", "/k/web-key", "/k/admin-key");
    let e = refused(files.read_access(&keys));
    assert_eq!(e.path, PathBuf::from("/k/access-key"));
    assert_eq!(e.error.kind(), io::ErrorKind::NotFound);
    assert_eq!(
        e.to_string(),
        format!("could not read /k/access-key: {}", e.error)
    );
    let e = refused(files.read_web(&keys));
    assert_eq!(e.path, PathBuf::from("/k/web-key"));
}

#[test]
fn a_missing_admin_key_or_name_file_is_none_but_an_unreadable_one_is_an_error() {
    let keys = keys("/k/access-key", "/k/web-key", "/k/admin-key");
    let files = Memory::new();
    assert!(files.read_admin(&keys).expect("missing").is_none());
    assert!(files.read_admin_names(&keys).expect("missing").is_none());
    let files = Memory::new().failing("/k/admin-key");
    assert_eq!(
        refused(files.read_admin(&keys)).path,
        PathBuf::from("/k/admin-key")
    );
    let files = Memory::new().failing("/k/admin-key.name");
    assert_eq!(
        refused(files.read_admin_names(&keys)).path,
        PathBuf::from("/k/admin-key.name")
    );
}

#[test]
fn a_new_key_makes_its_folder_before_the_key_is_drawn() {
    let files = Memory::new();
    let mut drawn = Vec::new();
    let key = files
        .new_key(Path::new("/k/sub/access-key"), false, &mut || {
            drawn.push(files.has_dir(Path::new("/k/sub")));
            "new".to_owned()
        })
        .expect("written");
    assert_eq!(key, "new");
    assert_eq!(drawn, [true]);
    let file = files.file(Path::new("/k/sub/access-key")).expect("file");
    assert_eq!(file.text, "new\n");
    assert_eq!(file.mode, Some(ACCESS_KEY_FILE_MODE));
}

#[test]
fn a_new_key_replaces_a_file_only_when_forced() {
    let files = Memory::new().with_file("/k/access-key", "old\n");
    let path = Path::new("/k/access-key");
    let refused = files.new_key(path, false, &mut || "new".to_owned());
    assert_eq!(
        refused,
        Err("/k/access-key already exists; pass --force to replace it".to_owned())
    );
    assert_eq!(files.file(path).expect("kept").text, "old\n");
    files
        .new_key(path, true, &mut || "new".to_owned())
        .expect("forced");
    assert_eq!(files.file(path).expect("replaced").text, "new\n");
}

#[test]
fn a_new_key_in_a_folder_that_cannot_be_made_draws_nothing() {
    let files = Memory::new().failing("/k");
    let mut drawn = false;
    let made = files.new_key(Path::new("/k/access-key"), false, &mut || {
        drawn = true;
        "new".to_owned()
    });
    assert!(made.is_err());
    assert!(!drawn);
}

#[test]
fn the_default_key_is_made_only_when_it_is_the_default_and_missing() {
    let path = PathBuf::from("/k/access-key");
    let mut draw = || "new".to_owned();
    let files = Memory::new();
    for other in [AccessKey::None, AccessKey::File(path.clone())] {
        assert_eq!(files.make_default_key(&other, &mut draw), Ok(None));
    }
    assert!(files.file(&path).is_none());
    assert_eq!(
        files.make_default_key(&AccessKey::Default(path.clone()), &mut draw),
        Ok(Some(path.clone()))
    );
    assert!(files.has_dir(Path::new("/k")));
    assert_eq!(files.file(&path).expect("made").text, "new\n");
    let files = Memory::new().with_file("/k/access-key", "old\n");
    assert_eq!(
        files.make_default_key(&AccessKey::Default(path.clone()), &mut draw),
        Ok(None)
    );
    assert_eq!(files.file(&path).expect("kept").text, "old\n");
}

#[test]
fn a_default_key_that_cannot_be_looked_at_is_refused_not_made_again() {
    let path = PathBuf::from("/k/access-key");
    let mut drawn = false;
    let mut draw = || {
        drawn = true;
        "new".to_owned()
    };
    let files = Memory::new()
        .with_file("/k/access-key", "old\n")
        .failing("/k/access-key");
    let made = files.make_default_key(&AccessKey::Default(path.clone()), &mut draw);
    let error = made.expect_err("refused");
    assert!(
        error.starts_with("could not tell whether /k/access-key is there: "),
        "{error}"
    );
    assert!(!drawn);
}

#[test]
fn the_admin_name_file_is_written_beside_its_key_or_removed() {
    let files = Memory::new().with_dir("/k");
    let key = Path::new("/k/admin-key");
    files.write_admin_name(key, Some("bound")).expect("written");
    assert_eq!(
        files
            .read_optional(Path::new("/k/admin-key.name"))
            .expect("read")
            .as_deref(),
        Some("bound")
    );
    files.write_admin_name(key, None).expect("removed");
    assert!(!files.try_exists(Path::new("/k/admin-key.name")).unwrap());
    files
        .write_admin_name(key, None)
        .expect("a missing one is fine");
}

#[test]
fn the_stand_in_like_the_disk_writes_only_in_a_folder_that_was_made() {
    let files = Memory::new();
    let path = Path::new("/k/access-key");
    let refused = files.write_key(path, "key", false).expect_err("no folder");
    assert!(
        refused.starts_with("could not write /k/access-key"),
        "{refused}"
    );
    assert!(
        files
            .write_admin_name(Path::new("/k/admin-key"), Some("n"))
            .is_err()
    );
    assert!(!files.try_exists(Path::new("/k")).unwrap());
    files.create_dir_all(Path::new("/k")).expect("made");
    files.write_key(path, "key", false).expect("written");
    assert!(files.try_exists(Path::new("/k")).unwrap());
}

#[test]
fn a_refused_move_leaves_the_staged_key_in_place() {
    let staged = Path::new("/k/client-key.partial");
    let files = Memory::new().with_file(staged, "key\n");
    let refused = files
        .keep_staged(staged, Path::new("/other/client-key"))
        .expect_err("no folder");
    assert!(
        refused.starts_with("could not move the key to /other/client-key"),
        "{refused}"
    );
    assert_eq!(files.file(staged).expect("still staged").text, "key\n");
    files
        .keep_staged(staged, Path::new("/k/client-key"))
        .expect("moved");
    assert!(files.file(staged).is_none());
    assert_eq!(
        files.file(Path::new("/k/client-key")).expect("kept").text,
        "key\n"
    );
}

#[test]
fn the_stand_in_gives_a_files_mode_or_says_it_cannot_look() {
    let files = Memory::new().with_dir("/k").with_file("/k/plain", "x");
    files
        .write_key(Path::new("/k/access-key"), "a key", false)
        .expect("written");
    assert_eq!(
        files.mode(Path::new("/k/access-key")).expect("mode"),
        Some(ACCESS_KEY_FILE_MODE)
    );
    assert_eq!(files.mode(Path::new("/k/plain")).expect("mode"), None);
    assert_eq!(
        files
            .mode(Path::new("/k/missing"))
            .expect_err("missing")
            .kind(),
        std::io::ErrorKind::NotFound
    );
    let refused = Memory::new()
        .with_file("/k/locked", "x")
        .failing("/k/locked");
    assert!(refused.mode(Path::new("/k/locked")).is_err());
}

#[test]
fn a_read_error_names_the_file_and_keeps_the_systems_reason() {
    let error = ReadError {
        path: PathBuf::from("/k/access-key"),
        error: io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
    };
    assert_eq!(error.to_string(), "could not read /k/access-key: denied");
    let source = std::error::Error::source(&error).expect("the io error");
    assert_eq!(source.to_string(), "denied");
}

/// Every message the stand-in's steps and the provided writers give,
/// pinned exactly: the container install's transcripts quote them.
#[test]
fn every_key_file_message_says_what_it_always_said() {
    let files = Memory::new().failing("/k");
    assert_eq!(
        files.create_dir_all(Path::new("/k")),
        Err("could not create /k: refused".to_owned())
    );
    let files = Memory::new();
    assert_eq!(
        files.write_key(Path::new("/k/access-key"), "key", false),
        Err("could not write /k/access-key: no such folder".to_owned())
    );
    let staged = Path::new("/k/client-key.partial");
    let files = Memory::new().with_file(staged, "key\n");
    assert_eq!(
        files.keep_staged(staged, Path::new("/other/client-key")),
        Err("could not move the key to /other/client-key: no such folder".to_owned())
    );
    let files = Memory::new()
        .with_dir("/k")
        .with_file("/k/access-key", "key\n");
    assert_eq!(
        files.new_key(Path::new("/k/access-key"), false, &mut || "new".into()),
        Err("/k/access-key already exists; pass --force to replace it".to_owned())
    );
    let files = Memory::new().with_dir("/k").failing("/k/access-key");
    assert_eq!(
        files.make_default_key(&AccessKey::Default("/k/access-key".into()), &mut || "new"
            .into()),
        Err("could not tell whether /k/access-key is there: refused".to_owned())
    );
}
