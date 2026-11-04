//! `kkt lanes export` and `kkt lanes import` with the real binary, on the
//! database of a separate server process while it runs: a round trip,
//! importing twice changing nothing, new versions for changed files,
//! retiring only with `--retire-missing`, the git commit of a checkout, a
//! README or a Markdown file with no lane header skipped with a warning
//! and its lane never retired, a bad lane file changing nothing, the
//! server's `max_lane_chars`, and the example lanes in `examples/lanes`.

#[path = "common/home.rs"]
mod home;
use kkt_scratch as scratch;
#[path = "common/serving.rs"]
mod serving;
use home::OwnHome;

mod common;

use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Output;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::process::{Child, Command};

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const TIMEOUT: Duration = Duration::from_secs(20);

static NEXT_HOME: AtomicU64 = AtomicU64::new(0);

fn temp_dir(name: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("kashkeshet-lane-files-{name}"))
}

/// Starts a server on `db` at a free port; returns it and its base URL once
/// it is listening.
async fn start_server(db: &Path) -> (Child, String) {
    let home = db.parent().expect("temp dir");
    let mut command = Command::new(KKT);
    command
        .arg("serve")
        .arg("--db")
        .arg(db)
        .arg("--no-access-key")
        .own_home(home);
    let server = serving::start(command).await;
    let base = server.base();
    (server.child, base)
}

/// Runs `kkt lanes` with `args`, in a home of its own with no config file
/// or setup record.
#[expect(
    clippy::disallowed_methods,
    reason = "kkt runs as a real process; this bounds the real wait so a hang fails the test"
)]
async fn kkt_lanes(args: &[&str]) -> Output {
    let home = temp_dir(&format!(
        "home-{}",
        NEXT_HOME.fetch_add(1, Ordering::Relaxed)
    ));
    let run = Command::new(KKT)
        .arg("lanes")
        .args(args)
        .own_home(&home)
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(TIMEOUT, run)
        .await
        .expect("kkt lanes should finish")
        .expect("run kkt lanes")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn arg(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

fn lane_file(name: &str, description: &str, text: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n{text}\n")
}

/// The lanes agents can take, as `list_lanes` gives them: name and
/// description.
async fn takeable(client: &common::Client) -> Vec<(String, String)> {
    let result = client
        .call_tool(
            CallToolRequestParams::new("list_lanes")
                .with_arguments(json!({ "format": "json" }).as_object().unwrap().clone()),
        )
        .await
        .expect("list_lanes");
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    let value: Value = serde_json::from_str(&text).expect("JSON");
    value["lanes"]
        .as_array()
        .expect("lanes")
        .iter()
        .map(|l| {
            (
                l["lane"].as_str().expect("lane").to_string(),
                l["description"].as_str().expect("description").to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn lanes_imported_while_the_server_runs_export_and_import_again_unchanged() {
    let dir = temp_dir("round-trip");
    let db = dir.join("kkt.db");
    let (mut server, base) = start_server(&db).await;
    let client = common::connect_new_at(&base).await;

    let lanes = dir.join("lanes");
    std::fs::create_dir_all(&lanes).expect("lanes folder");
    std::fs::write(
        lanes.join("reviewer.md"),
        lane_file(
            "reviewer",
            "Reviews pull requests.",
            "Read the diff.\n\nTwice.",
        ),
    )
    .expect("write");
    std::fs::write(
        lanes.join("writer.md"),
        lane_file("writer", "Writes the docs.", "Plain words."),
    )
    .expect("write");
    // Instructions that end with a carriage return: the file's final
    // `\r\n` is its line end, so the stored text is "Close it.\r".
    std::fs::write(
        lanes.join("closer.md"),
        "---\nname: closer\ndescription: Closes issues.\n---\n\nClose it.\r\r\n",
    )
    .expect("write");
    std::fs::write(lanes.join("notes.txt"), "not a lane").expect("write");

    let out = kkt_lanes(&["import", arg(&lanes), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    let said = stdout(&out);
    assert!(said.contains("created closer (version 1)\n"), "{said}");
    assert!(said.contains("created reviewer (version 1)\n"), "{said}");
    assert!(said.contains("created writer (version 1)\n"), "{said}");
    assert!(
        said.contains(&format!(
            "Imported from {}: 3 created, 0 updated, 0 unchanged",
            lanes.display()
        )),
        "{said}"
    );
    // The running server offers them at once.
    assert_eq!(
        takeable(&client).await,
        [
            ("closer".to_string(), "Closes issues.".to_string()),
            ("reviewer".to_string(), "Reviews pull requests.".to_string()),
            ("writer".to_string(), "Writes the docs.".to_string()),
        ]
    );

    // Exported, they come out as they went in.
    let exported = dir.join("exported");
    let out = kkt_lanes(&["export", arg(&exported), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        stdout(&out),
        "closer.md (version 1)\nreviewer.md (version 1)\nwriter.md (version 1)\n"
    );
    for name in ["closer.md", "reviewer.md", "writer.md"] {
        assert_eq!(
            std::fs::read_to_string(exported.join(name)).expect("exported"),
            std::fs::read_to_string(lanes.join(name)).expect("imported"),
            "{name}"
        );
    }

    // Importing the export changes nothing.
    let out = kkt_lanes(&["import", arg(&exported), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    let said = stdout(&out);
    assert!(said.contains("unchanged closer (version 1)\n"), "{said}");
    assert!(said.contains("unchanged reviewer (version 1)\n"), "{said}");
    assert!(said.contains("0 created, 0 updated, 3 unchanged"), "{said}");

    // A changed file gets a new version; a missing one is retired only
    // with --retire-missing.
    std::fs::write(
        exported.join("reviewer.md"),
        lane_file("reviewer", "Reviews pull requests.", "Read the diff once."),
    )
    .expect("write");
    std::fs::remove_file(exported.join("writer.md")).expect("remove");
    let out = kkt_lanes(&["import", arg(&exported), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        stdout(&out).contains("updated reviewer (version 2)\n"),
        "{}",
        stdout(&out)
    );
    assert_eq!(takeable(&client).await.len(), 3);
    let out = kkt_lanes(&[
        "import",
        arg(&exported),
        "--retire-missing",
        "--db",
        arg(&db),
    ])
    .await;
    assert!(out.status.success(), "{}", text(&out));
    let said = stdout(&out);
    assert!(
        said.contains("retired writer: it has no file in the folder\n"),
        "{said}"
    );
    assert!(said.contains(", 1 retired"), "{said}");
    assert_eq!(
        takeable(&client).await,
        [
            ("closer".to_string(), "Closes issues.".to_string()),
            ("reviewer".to_string(), "Reviews pull requests.".to_string())
        ]
    );
    // A retired lane is not exported.
    let again = dir.join("again");
    let out = kkt_lanes(&["export", arg(&again), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        stdout(&out),
        "closer.md (version 1)\nreviewer.md (version 2)\n"
    );
    assert!(!again.join("writer.md").exists());

    client.cancel().await.expect("close");
    server.kill().await.expect("stop server");
}

#[tokio::test]
async fn the_example_lanes_import_and_export_unchanged() {
    let dir = temp_dir("examples");
    let db = dir.join("kkt.db");
    let (mut server, base) = start_server(&db).await;
    let client = common::connect_new_at(&base).await;
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/lanes");

    let out = kkt_lanes(&["import", arg(&examples), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    let said = stdout(&out);
    assert!(said.contains("3 created, 0 updated, 0 unchanged"), "{said}");
    let names: Vec<String> = takeable(&client)
        .await
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["orchestrator", "planner", "tester"]);

    let exported = dir.join("exported");
    let out = kkt_lanes(&["export", arg(&exported), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    for name in ["orchestrator.md", "planner.md", "tester.md"] {
        assert_eq!(
            std::fs::read_to_string(exported.join(name)).expect("exported"),
            std::fs::read_to_string(examples.join(name)).expect("example"),
            "{name}"
        );
    }

    client.cancel().await.expect("close");
    server.kill().await.expect("stop server");
}

/// Runs git in `dir`, with a home of its own so no real git config is read.
fn git(dir: &Path, home: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .own_home(home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn a_folder_in_a_git_checkout_records_its_commit_and_whether_it_was_changed() {
    let dir = temp_dir("git");
    let db = dir.join("kkt.db");
    let (mut server, _base) = start_server(&db).await;
    let repo = dir.join("repo");
    let lanes = repo.join("lanes");
    std::fs::create_dir_all(&lanes).expect("lanes folder");
    std::fs::write(
        lanes.join("reviewer.md"),
        lane_file("reviewer", "Reviews.", "Read."),
    )
    .expect("write");
    git(&repo, &dir, &["init", "--quiet"]);
    git(&repo, &dir, &["add", "."]);
    git(
        &repo,
        &dir,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "lanes",
        ],
    );
    let head = git(&repo, &dir, &["rev-parse", "HEAD"]);

    let out = kkt_lanes(&["import", arg(&lanes), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        stdout(&out).contains(&format!(
            "Imported from {} (git commit {head}): 1 created",
            lanes.display()
        )),
        "{}",
        stdout(&out)
    );

    // A change not committed yet is marked.
    std::fs::write(
        lanes.join("reviewer.md"),
        lane_file("reviewer", "Reviews.", "Read twice."),
    )
    .expect("write");
    let out = kkt_lanes(&["import", arg(&lanes), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        stdout(&out).contains(&format!("(git commit {head}-dirty): 0 created, 1 updated")),
        "{}",
        stdout(&out)
    );
    server.kill().await.expect("stop server");
}

#[tokio::test]
async fn a_readme_or_a_markdown_file_with_no_lane_header_is_skipped_with_a_warning() {
    let dir = temp_dir("skipped");
    let db = dir.join("kkt.db");
    let (mut server, base) = start_server(&db).await;
    let client = common::connect_new_at(&base).await;
    let lanes = dir.join("lanes");
    std::fs::create_dir_all(&lanes).expect("lanes folder");
    std::fs::write(
        lanes.join("reviewer.md"),
        lane_file("reviewer", "Reviews.", "Read."),
    )
    .expect("write");
    std::fs::write(lanes.join("README.md"), "# Our lanes\n").expect("write");
    std::fs::write(lanes.join("notes.md"), "Notes, not a lane.\n").expect("write");

    let out = kkt_lanes(&["import", arg(&lanes), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        stdout(&out).contains("1 created, 0 updated, 0 unchanged"),
        "{}",
        stdout(&out)
    );
    // One warning per skipped file, and none on standard output.
    let warned = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        warned.contains(&format!(
            "warning: skipped {}: a README is not a lane\n",
            lanes.join("README.md").display()
        )),
        "{warned}"
    );
    assert!(
        warned.contains(&format!(
            "warning: skipped {}: it does not start with a lane header",
            lanes.join("notes.md").display()
        )),
        "{warned}"
    );
    assert_eq!(warned.matches("warning:").count(), 2, "{warned}");
    assert!(!stdout(&out).contains("warning:"), "{}", stdout(&out));
    assert_eq!(
        takeable(&client).await,
        [("reviewer".to_string(), "Reviews.".to_string())]
    );

    // A lane's own file that loses its header is skipped too, and
    // --retire-missing leaves that lane alone instead of retiring it.
    std::fs::write(
        lanes.join("writer.md"),
        lane_file("writer", "Writes.", "Write."),
    )
    .expect("write");
    let out = kkt_lanes(&["import", arg(&lanes), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    std::fs::write(
        lanes.join("writer.md"),
        format!("\n{}", lane_file("writer", "Writes.", "Write.")),
    )
    .expect("write");
    let out = kkt_lanes(&["import", arg(&lanes), "--retire-missing", "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    let said = stdout(&out);
    assert!(
        said.contains("left writer alone: its file was skipped, so it is not retired\n"),
        "{said}"
    );
    assert!(said.contains(", 1 left alone for a skipped file"), "{said}");
    assert!(!said.contains("retired writer"), "{said}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&format!(
            "warning: skipped {}: it does not start",
            lanes.join("writer.md").display()
        )),
        "{}",
        text(&out)
    );
    assert_eq!(
        takeable(&client).await,
        [
            ("reviewer".to_string(), "Reviews.".to_string()),
            ("writer".to_string(), "Writes.".to_string()),
        ]
    );

    // --retire-missing on a folder of notes alone, such as the wrong one,
    // is refused, so it cannot retire every lane.
    let notes = dir.join("notes");
    std::fs::create_dir_all(&notes).expect("notes folder");
    std::fs::write(notes.join("README.md"), "# Notes\n").expect("write");
    let out = kkt_lanes(&["import", arg(&notes), "--retire-missing", "--db", arg(&db)]).await;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains(&format!(
            "{} has no lane files, only skipped .md files",
            notes.display()
        )),
        "{}",
        text(&out)
    );
    assert_eq!(takeable(&client).await.len(), 2);

    client.cancel().await.expect("close");
    server.kill().await.expect("stop server");
}

#[tokio::test]
async fn a_bad_file_or_a_lane_over_the_limit_imports_nothing() {
    let dir = temp_dir("bad");
    let db = dir.join("kkt.db");
    let (mut server, _base) = start_server(&db).await;
    let lanes = dir.join("lanes");
    std::fs::create_dir_all(&lanes).expect("lanes folder");
    std::fs::write(
        lanes.join("reviewer.md"),
        lane_file("reviewer", "Reviews.", "Read."),
    )
    .expect("write");
    // A file with a lane header is a lane file, so a mistake in it is
    // refused, not skipped.
    std::fs::write(lanes.join("broken.md"), "---\nname: broken\n").expect("write");
    let out = kkt_lanes(&["import", arg(&lanes), "--db", arg(&db)]).await;
    assert!(!out.status.success());
    assert!(
        text(&out).contains(&format!(
            "{}: the header has no closing ---",
            lanes.join("broken.md").display()
        )),
        "{}",
        text(&out)
    );
    std::fs::remove_file(lanes.join("broken.md")).expect("remove");

    // The config file's max_lane_chars limits the instructions.
    std::fs::write(
        lanes.join("writer.md"),
        lane_file("writer", "Writes.", &"x".repeat(101)),
    )
    .expect("write");
    let config = dir.join("config.toml");
    std::fs::write(&config, "max_lane_chars = 100\n").expect("config");
    let out = kkt_lanes(&[
        "import",
        arg(&lanes),
        "--db",
        arg(&db),
        "--config",
        arg(&config),
    ])
    .await;
    assert!(!out.status.success());
    assert!(
        text(&out).contains("lane writer: the lane's instructions must be at most 100"),
        "{}",
        text(&out)
    );

    // Nothing was imported, the good lane included.
    let out = kkt_lanes(&["export", arg(&dir.join("out")), "--db", arg(&db)]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(stdout(&out), "");
    server.kill().await.expect("stop server");
}

#[tokio::test]
async fn a_missing_database_or_folder_is_an_error() {
    let dir = temp_dir("missing");
    let db = dir.join("nothing-here.db");
    let out = kkt_lanes(&["export", arg(&dir.join("out")), "--db", arg(&db)]).await;
    assert!(!out.status.success());
    assert!(text(&out).contains("no database at"), "{}", text(&out));
    assert!(!db.exists());

    let db = dir.join("kkt.db");
    let (mut server, _base) = start_server(&db).await;
    let out = kkt_lanes(&["import", arg(&dir.join("no-lanes")), "--db", arg(&db)]).await;
    assert!(!out.status.success());
    assert!(
        text(&out).contains("could not read the folder"),
        "{}",
        text(&out)
    );
    server.kill().await.expect("stop server");
}
