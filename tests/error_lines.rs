//! What each `kkt` command says when it fails, and how it exits: a plain
//! `error: ...` line on standard error (with empty standard output) and exit
//! code 1. Each command is run with a home of its own; a failure that does
//! not depend on the machine is chosen for each.

#[path = "common/home.rs"]
mod home;
use home::OwnHome;
use kkt_scratch as scratch;
use std::path::Path;
use std::process::{Command, Stdio};

const KKT: &str = env!("CARGO_BIN_EXE_kkt");

/// What `kkt args` printed on standard output and standard error, and its
/// exit code.
fn failed(home: &Path, args: &[&str]) -> (Vec<String>, Vec<String>, Option<i32>) {
    let out = Command::new(KKT)
        .args(args)
        .own_home(home)
        .stdin(Stdio::null())
        .output()
        .expect("run kkt");
    let lines = |bytes: &[u8]| -> Vec<String> {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(ToString::to_string)
            .collect()
    };
    (lines(&out.stdout), lines(&out.stderr), out.status.code())
}

#[test]
fn every_command_says_why_it_failed_and_exits_with_one() {
    let home = scratch::Dir::new("error-lines");
    let printed = |args: &[&str]| failed(&home, args);
    let on_stderr = |line: &str| (Vec::new(), vec![format!("error: {line}")], Some(1));
    let missing_db = "no database at /nope/kkt.db; pass --db with the server's database file";

    assert_eq!(
        printed(&["key", "header", "--key-file", "/nope/key"]),
        on_stderr("could not read key file /nope/key: No such file or directory (os error 2)")
    );
    assert_eq!(
        printed(&["key", "show", "--web"]),
        on_stderr(&format!(
            "there is no web key at {}; pass --key-file <path> if it is elsewhere, or run `kkt \
             key create --web` to make one",
            home.join(".config/kkt/web-key").display()
        ))
    );
    assert_eq!(
        printed(&["agent", "list", "--db", "/nope/kkt.db"]),
        on_stderr(missing_db)
    );
    assert_eq!(
        printed(&["lanes", "export", "out", "--db", "/nope/kkt.db"]),
        on_stderr(missing_db)
    );
    let backups = home.join("backups");
    assert_eq!(
        printed(&[
            "backup",
            &backups.display().to_string(),
            "--db",
            "/nope/kkt.db"
        ]),
        on_stderr(missing_db)
    );
    assert_eq!(
        printed(&["switch", "--local"]),
        on_stderr("this machine is not set up yet; run `kkt setup` first")
    );
    assert_eq!(
        printed(&["setup", "--clients", "bogus"]),
        on_stderr(
            "unknown client \"bogus\" in --clients: use claude-code, codex, opencode, agy, \
             muse-code or none"
        )
    );
    let key = home.join("k");
    assert_eq!(
        printed(&[
            "setup",
            "--mode",
            "client",
            "--url",
            "ftp://x",
            "--key-file",
            &key.display().to_string(),
            "--clients",
            "none"
        ]),
        (
            Vec::new(),
            vec![
                "error: the answer given with --url did not pass its check, so nothing was \
                 changed:"
                    .to_string(),
                "URL must be an absolute http:// or https:// address".to_string()
            ],
            Some(1)
        )
    );
    // The agents' commands keep standard output for their results.
    let token = home.join("token");
    assert_eq!(
        printed(&[
            "register",
            "--url",
            "http://127.0.0.1:9/mcp",
            "--token-file",
            &token.display().to_string()
        ]),
        on_stderr(&format!(
            "could not read key file {}: No such file or directory (os error 2)",
            home.join(".config/kkt/access-key").display()
        ))
    );

    // `kkt upgrade` on a client machine refuses plainly on stderr.
    let config = home.join(".config/kkt");
    std::fs::create_dir_all(&config).expect("config folder");
    std::fs::write(
        config.join("setup.toml"),
        "format = 1\nmode = \"client\"\nwritten_by = \"1.0.0\"\nurl = \"http://127.0.0.1:7420/mcp\"\n",
    )
    .expect("client record");
    assert_eq!(
        printed(&["upgrade"]),
        on_stderr("this machine is a client; upgrade the server on the machine where it runs")
    );

    // A setup record that cannot be read stops each command that reads it.
    std::fs::write(config.join("setup.toml"), "garbage").expect("a bad record");
    let bad_record = format!(
        "the setup record {} is not valid: key with no value, expected `=`",
        config.join("setup.toml").display()
    );
    for args in [
        &["status"][..],
        &["upgrade"],
        &["uninstall"],
        &["doctor"],
        &["serve"],
        &["key", "create", "--admin"],
        &["lanes", "export", "out"],
        &["backup", &backups.display().to_string()],
    ] {
        assert_eq!(printed(args), on_stderr(&bad_record), "{args:?}");
    }

    // A config file that cannot be read stops the server before it starts.
    std::fs::remove_file(config.join("setup.toml")).expect("no record");
    std::fs::write(config.join("config.toml"), "bind = 5\n").expect("a bad config");
    assert_eq!(
        printed(&["serve"]),
        on_stderr(&format!(
            "invalid setting: bind in the config file must be an address in quotes, such as \
             \"127.0.0.1:7420\", not 5 (in {})",
            config.join("config.toml").display()
        ))
    );
}
