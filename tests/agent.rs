//! `kkt agent`: the operator lists agents and revokes a token in the
//! database file while a separate server process runs on it.

#[path = "common/home.rs"]
mod home;
use kkt_scratch as scratch;
#[path = "common/serving.rs"]
mod serving;
use home::OwnHome;

mod common;

use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::path::Path;
use std::process::Output;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::process::{Child, Command};

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const TIMEOUT: Duration = Duration::from_secs(20);

static NEXT_HOME: AtomicU64 = AtomicU64::new(0);

fn temp_dir(name: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("kashkeshet-agent-{name}"))
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

async fn kkt_agent(args: &[&str]) -> Output {
    // A home with no config file in it, so only the arguments decide. It is
    // removed when this returns, after the command has finished.
    let home = temp_dir(&format!(
        "home-{}",
        NEXT_HOME.fetch_add(1, Ordering::Relaxed)
    ));
    let run = Command::new(KKT)
        .arg("agent")
        .args(args)
        .own_home(&home)
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(TIMEOUT, run)
        .await
        .expect("kkt agent should finish")
        .expect("run kkt agent")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Calls `whoami` with `token`; returns the text and whether it was an error.
async fn whoami(client: &common::Client, token: &str) -> (String, bool) {
    let params = CallToolRequestParams::new("whoami").with_arguments(
        json!({ "token": token })
            .as_object()
            .expect("object")
            .clone(),
    );
    let result = client.call_tool(params).await.expect("call whoami");
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    (text, result.is_error == Some(true))
}

#[tokio::test]
async fn revoking_while_the_server_runs_refuses_the_next_call() {
    let dir = temp_dir("revoke");
    let db = dir.join("kkt.db");
    let db_arg = db.to_str().expect("utf-8 path");
    let (mut server, base) = start_server(&db).await;
    let client = common::connect_new_at(&base).await;

    let registered = client
        .call_tool(CallToolRequestParams::new("register"))
        .await
        .expect("register");
    let reply: String = registered
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    let field = |label: &str| {
        reply
            .lines()
            .find_map(|l| l.strip_prefix(label))
            .expect("field")
            .trim()
            .to_string()
    };
    let name = field("handle:");
    let token = field("token:");
    let (me, refused) = whoami(&client, &token).await;
    assert!(!refused, "{me}");

    let out = kkt_agent(&["revoke", &name, "--db", db_arg]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains(&format!("revoked {name}")));

    let (answer, refused) = whoami(&client, &token).await;
    assert!(refused, "{answer}");
    assert!(answer.contains("unauthenticated"), "{answer}");

    let out = kkt_agent(&["list", "--db", db_arg]).await;
    assert!(out.status.success(), "{}", text(&out));
    let listed = text(&out);
    let line = listed
        .lines()
        .find(|l| l.starts_with(&name))
        .expect("agent listed");
    assert!(line.contains("revoked"), "{line}");

    let out = kkt_agent(&["revoke", &name, "--db", db_arg]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("already revoked"), "{}", text(&out));

    let out = kkt_agent(&["revoke", "no-such-agent", "--db", db_arg]).await;
    assert!(!out.status.success());
    assert!(text(&out).contains("not_found"), "{}", text(&out));

    client.cancel().await.expect("close");
    server.kill().await.expect("stop server");
}

/// Calls `tool` with `args` as JSON; returns the reply and whether it was an
/// error.
async fn call_json(
    client: &common::Client,
    tool: &'static str,
    args: serde_json::Value,
) -> (serde_json::Value, bool) {
    let mut args = args.as_object().expect("object").clone();
    args.insert("format".into(), json!("json"));
    let result = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
        .await
        .expect("call");
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    (
        serde_json::from_str(&text).expect("JSON"),
        result.is_error == Some(true),
    )
}

/// A database written by v0.7.0, when avatars' handles were still called
/// names: one agent with a known token and recovery code, in one channel.
fn make_v070(path: &Path) {
    use sha2::{Digest, Sha256};
    let conn = rusqlite::Connection::open(path).expect("open");
    conn.pragma_update(None, "foreign_keys", "OFF")
        .expect("foreign keys");
    for migration in [
        include_str!("../crates/kkt-store/migrations/0001_init.sql"),
        include_str!("../crates/kkt-store/migrations/0002_server_runs.sql"),
        include_str!("../crates/kkt-store/migrations/0003_revoked_agents.sql"),
        include_str!("../crates/kkt-store/migrations/0004_identity_recovery.sql"),
        include_str!("../crates/kkt-store/migrations/0005_messages_by_time.sql"),
        include_str!("../crates/kkt-store/migrations/0006_agent_descriptions.sql"),
        include_str!("../crates/kkt-store/migrations/0007_channel_notes.sql"),
        include_str!("../crates/kkt-store/migrations/0008_operator_messages.sql"),
        include_str!("../crates/kkt-store/migrations/0009_handoff_notes.sql"),
        include_str!("../crates/kkt-store/migrations/0010_hidden_entries.sql"),
        include_str!("../crates/kkt-store/migrations/0011_operator_instructions.sql"),
        include_str!("../crates/kkt-store/migrations/0012_operator_channels.sql"),
        include_str!("../crates/kkt-store/migrations/0013_channel_owners.sql"),
        include_str!("../crates/kkt-store/migrations/0014_agent_clients.sql"),
        include_str!("../crates/kkt-store/migrations/0015_instruction_authors.sql"),
    ] {
        conn.execute_batch(migration).expect("migration");
    }
    conn.pragma_update(None, "user_version", 15)
        .expect("version");
    conn.execute(
        "INSERT INTO agents (name, token_hash, recovery_hash, created_at) VALUES ('amber-falcon', ?1, ?2, 't')",
        (
            Sha256::digest(b"act_oldtoken").to_vec(),
            Sha256::digest(b"arc_oldrecovery").to_vec(),
        ),
    )
    .expect("agent");
    conn.execute_batch(
        "INSERT INTO channels (id, name, is_open, created_by, created_at, owner)
             VALUES ('ch_1', 'general', 0, 'amber-falcon', 't', 'amber-falcon');
         INSERT INTO memberships VALUES ('ch_1', 'amber-falcon', 't');",
    )
    .expect("channel");
}

#[tokio::test]
async fn an_avatar_from_a_v0_7_0_database_is_reached_by_its_handle() {
    let dir = temp_dir("v070-handle");
    let db = dir.join("old.db");
    make_v070(&db);
    let (mut server, base) = start_server(&db).await;
    let client = common::connect_new_at(&base).await;

    // The token it was given still works, and names it by its handle.
    let (me, refused) = call_json(&client, "whoami", json!({ "token": "act_oldtoken" })).await;
    assert!(!refused, "{me}");
    assert_eq!(me["handle"], "amber-falcon");
    // So does its recovery code, given with its handle.
    let (reclaimed, refused) = call_json(
        &client,
        "reclaim",
        json!({ "handle": "amber-falcon", "recovery_code": "arc_oldrecovery" }),
    )
    .await;
    assert!(!refused, "{reclaimed}");
    assert_eq!(reclaimed["handle"], "amber-falcon");
    let token = reclaimed["token"].as_str().expect("token");
    // Others find it, and its channel, by its handle.
    let (members, refused) = call_json(
        &client,
        "list_members",
        json!({ "token": token, "channel_id": "ch_1" }),
    )
    .await;
    assert!(!refused, "{members}");
    assert_eq!(members["members"][0]["handle"], "amber-falcon");
    assert_eq!(members["owner"], "amber-falcon");
    let (agents, refused) = call_json(
        &client,
        "list_agents",
        json!({ "token": token, "query": "amber" }),
    )
    .await;
    assert!(!refused, "{agents}");
    assert_eq!(agents["agents"][0]["handle"], "amber-falcon");

    // A new avatar may not take its handle.
    let (taken, refused) =
        call_json(&client, "register", json!({ "handle": "amber-falcon" })).await;
    assert!(refused, "{taken}");
    assert_eq!(taken["error"]["code"], "handle_taken");

    // `kkt agent` finds it by its handle too.
    let out = kkt_agent(&["list", "--db", db.to_str().expect("utf-8 path")]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("amber-falcon"), "{}", text(&out));

    client.cancel().await.expect("close");
    server.kill().await.expect("stop server");
}

#[tokio::test]
async fn a_missing_database_is_not_created() {
    let dir = temp_dir("missing");
    let db = dir.join("nothing-here.db");
    let out = kkt_agent(&["list", "--db", db.to_str().expect("utf-8 path")]).await;
    assert!(!out.status.success());
    assert!(text(&out).contains("no database at"), "{}", text(&out));
    assert!(!db.exists());
}

#[tokio::test]
async fn an_explicit_database_does_not_read_the_config_file() {
    let dir = temp_dir("explicit");
    let db = dir.join("given.db");
    let (mut server, _base) = start_server(&db).await;
    server.kill().await.expect("stop server");
    let config = dir.join("config.toml");
    std::fs::write(&config, "not = [valid toml\n").expect("write config");
    let out = kkt_agent(&[
        "list",
        "--db",
        db.to_str().expect("utf-8 path"),
        "--config",
        config.to_str().expect("utf-8 path"),
    ])
    .await;
    assert!(out.status.success(), "{}", text(&out));
}

#[tokio::test]
async fn the_database_is_found_through_the_config_file() {
    let dir = temp_dir("config");
    let db = dir.join("from-config.db");
    let (mut server, _base) = start_server(&db).await;
    server.kill().await.expect("stop server");
    // `db` in the config file is taken from the config file's folder.
    let config = dir.join("config.toml");
    std::fs::write(&config, "db = \"from-config.db\"\n").expect("write config");
    let out = kkt_agent(&["list", "--config", config.to_str().expect("utf-8 path")]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains(&format!("no agents in {}", db.display())),
        "{}",
        text(&out)
    );
}
