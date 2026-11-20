//! What each `kkt` command says when it fails, and how it exits: plain text
//! on standard error starting with `error: ` (no timestamp and no `ERROR`
//! log level) and exit code 1. A message may span more than one line where
//! it already did, and a command may have already printed completed work on
//! standard output before failing; the cases here fail before printing
//! anything else, so their standard output is empty. Each command is run
//! with a home of its own; a failure that does not depend on the machine is
//! chosen for each.

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
        on_stderr("there is no recorded local server for --local; run `kkt setup` first")
    );
    assert_eq!(
        printed(&[
            "switch",
            "https://example.invalid/mcp?demo=1",
            "--clients",
            "none"
        ]),
        on_stderr(
            "put credentials in the key file, not in the URL; query strings and fragments are not supported"
        )
    );
    assert_eq!(
        printed(&[
            "switch",
            "https://example.invalid/mcp",
            "--key-file",
            "/nope/key"
        ]),
        on_stderr(
            "this machine has no setup record; run `kkt setup --mode client --url \
             'https://example.invalid/mcp' --key-file '/nope/key' --clients none --no-allow-kkt`"
        )
    );
    assert_eq!(
        printed(&["setup"]),
        on_stderr("no terminal to ask what this machine is for; pass --mode")
    );
    assert_eq!(
        printed(&["setup", "--mode", "client"]),
        on_stderr("no terminal to ask for the server's MCP address; pass --url")
    );
    assert_eq!(
        printed(&["connect", "codex", "--url", "ftp://x"]),
        on_stderr("URL must be an absolute http:// or https:// address")
    );
    assert_eq!(
        printed(&["key", "create", "--web", "--name", "sentinel"]),
        on_stderr("--name names the admin agent, so it goes with --admin")
    );
    assert_eq!(
        printed(&[
            "reclaim",
            "--handle",
            "amber-falcon",
            "--recovery-file",
            "/nope/recovery",
            "--token-file",
            &home.join("reclaim-token").display().to_string(),
            "--url",
            "http://127.0.0.1:9/mcp",
        ]),
        on_stderr(
            "could not inspect credential file /nope/recovery: No such file or directory (os error 2)"
        )
    );
    assert_eq!(
        printed(&["watch", "--token-file", "/nope/token", "--url", "ftp://x"]),
        on_stderr(
            "could not inspect credential file /nope/token: No such file or directory (os error 2)"
        )
    );
    assert_eq!(
        printed(&["service", "install", "--config", "/nope/config.toml"]),
        on_stderr(
            "invalid setting: config could not be read from /nope/config.toml: No such file or \
             directory (os error 2)"
        )
    );
    // A `--clients` name checked after the parser is wrong usage: the
    // same plain line, but exit 2, as `tests/exit_codes.rs` pins.
    assert_eq!(
        printed(&["setup", "--clients", "bogus"]),
        (
            Vec::new(),
            vec![
                "error: unknown client \"bogus\" in --clients: use claude-code, codex, \
                 opencode, agy, muse-code or none"
                    .to_string()
            ],
            Some(2)
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

#[cfg(unix)]
#[test]
fn switch_cannot_suggest_a_different_key_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let home = scratch::Dir::new("switch-key-path");
    let out = Command::new(KKT)
        .args([
            "switch",
            "https://example.invalid/mcp",
            "--clients",
            "none",
            "--key-file",
        ])
        .arg(OsStr::from_bytes(b"/tmp/key-\xff"))
        .own_home(&home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(out.stderr, b"error: the key path is not UTF-8\n");
}
