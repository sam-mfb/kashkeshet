//! The `kkt` command's help is part of what users rely on. This test runs
//! `kkt --help` and the `--help` of every command and nested command, and
//! compares each with its snapshot in `tests/help/`, so any change to the
//! help shows up as a failure.
//!
//! The commands are found by walking the tree: each help page's `Commands:`
//! list names the commands to visit next. So a new command is pinned as soon
//! as it exists, and a snapshot whose command is gone is reported.
//!
//! After an intended change, rewrite the snapshots with
//! `UPDATE_HELP=1 cargo test --test help` and review the diff. That also
//! removes the snapshots of commands that no longer exist.

#[path = "common/home.rs"]
mod home;
#[path = "common/scratch.rs"]
mod scratch;
use home::OwnHome;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const UPDATE_VAR: &str = "UPDATE_HELP";
const SNAPSHOT_EXTENSION: &str = "txt";

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/help")
}

/// The snapshot file for a command path: `kkt.txt` for `kkt` itself,
/// `kkt-service-install.txt` for `kkt service install`.
fn snapshot_name(path: &[String]) -> String {
    let mut name = String::from("kkt");
    for part in path {
        name.push('-');
        name.push_str(part);
    }
    format!("{name}.{SNAPSHOT_EXTENSION}")
}

/// `kkt <path> --help`, with an empty environment apart from a HOME of its
/// own, so nothing on this machine can change the text.
fn help_text(home: &Path, path: &[String]) -> String {
    let out = Command::new(KKT)
        .env_clear()
        .own_home(home)
        .args(path)
        .arg("--help")
        .output()
        .expect("run kkt");
    let shown = format!("kkt {} --help", path.join(" "));
    assert!(
        out.status.success(),
        "{shown} failed: {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{shown} wrote to standard error:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("help is UTF-8")
}

/// The commands a help page lists under `Commands:`, without clap's own
/// `help` command.
fn listed_commands(help: &str) -> Vec<String> {
    help.lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter(|line| line.starts_with("  ") && !line[2..].starts_with(' '))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_string)
        .collect()
}

/// Every help page, by snapshot file name, found by walking the commands.
fn every_help_page(home: &Path) -> BTreeMap<String, String> {
    let mut pages = BTreeMap::new();
    let mut to_visit: Vec<Vec<String>> = vec![Vec::new()];
    while let Some(path) = to_visit.pop() {
        let text = help_text(home, &path);
        for command in listed_commands(&text) {
            let mut next = path.clone();
            next.push(command);
            to_visit.push(next);
        }
        pages.insert(snapshot_name(&path), text);
    }
    pages
}

/// The snapshot files now in `tests/help/`.
fn snapshot_files(dir: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => panic!("could not read {}: {e}", dir.display()),
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.expect("read snapshot folder entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == SNAPSHOT_EXTENSION)
        })
        .map(|path| {
            path.file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn every_help_page_matches_its_snapshot() {
    let home = scratch::Dir::new("help-pages");
    let pages = every_help_page(home.path());
    let dir = snapshot_dir();

    if std::env::var_os(UPDATE_VAR).is_some() {
        std::fs::create_dir_all(&dir).expect("make snapshot folder");
        for name in snapshot_files(&dir) {
            if !pages.contains_key(&name) {
                std::fs::remove_file(dir.join(&name)).expect("remove old snapshot");
            }
        }
        for (name, text) in &pages {
            std::fs::write(dir.join(name), text).expect("write snapshot");
        }
        return;
    }

    let stale: Vec<String> = snapshot_files(&dir)
        .into_iter()
        .filter(|name| !pages.contains_key(name))
        .collect();
    assert!(
        stale.is_empty(),
        "these snapshots have no command: {stale:?}. If that was intended, run \
         `{UPDATE_VAR}=1 cargo test --test help` and review the diff."
    );

    let mut changed = Vec::new();
    for (name, actual) in &pages {
        let path = dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(expected) if expected == *actual => {}
            Ok(_) => changed.push(format!("{name} changed. actual:\n{actual}")),
            Err(e) => changed.push(format!(
                "could not read {}: {e}. Create it with {UPDATE_VAR}=1",
                path.display()
            )),
        }
    }
    assert!(
        changed.is_empty(),
        "the help of {} command(s) changed. If that was intended, run \
         `{UPDATE_VAR}=1 cargo test --test help` and review the diff.\n\n{}",
        changed.len(),
        changed.join("\n\n")
    );
}

/// The walk must find nested commands, or the test would pin only the top.
#[test]
fn the_walk_reaches_nested_commands() {
    let home = scratch::Dir::new("help-walk");
    let pages = every_help_page(home.path());
    for name in ["kkt.txt", "kkt-serve.txt", "kkt-service-install.txt"] {
        assert!(
            pages.contains_key(name),
            "{name} not found in {:?}",
            pages.keys()
        );
    }
}
