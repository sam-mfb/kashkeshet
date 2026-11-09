//! Taking lanes over MCP and with `kkt register --lane`: listing them without
//! a token, registering with one, the marked section after the server
//! announcement, a newer version shown to the agent and marked to others and
//! at `/mcp-admin`, `take_lane`'s changes announced in the agent's channels,
//! and refusals that change nothing.

mod common;
use kkt_scratch as scratch;

use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const ACCESS_KEY: &str = "access-0123456789abcdef0123456789abcdef";
const ADMIN_KEY: &str = "admin-0123456789abcdef0123456789abcdef";

/// A server with an access key and an admin key named `admin-sentinel`.
async fn start() -> common::Server {
    let mut config = common::test_config();
    config.access_key = Some(ACCESS_KEY.into());
    config.admin_key = Some(ADMIN_KEY.into());
    config.admin_name = Some("sentinel".into());
    common::start_with(config).await
}

/// The tool's reply as text, and whether it is an error.
async fn call_raw(client: &common::Client, tool: &str, arguments: Value) -> (String, bool) {
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(arguments))
        .await
        .expect(tool);
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    (text, result.is_error == Some(true))
}

/// The tool's JSON reply, and whether it is an error.
async fn call(client: &common::Client, tool: &str, arguments: Value) -> (Value, bool) {
    let mut arguments = arguments.as_object().cloned().unwrap_or_default();
    arguments.insert("format".into(), json!("json"));
    let (text, error) = call_raw(client, tool, Value::Object(arguments)).await;
    (serde_json::from_str(&text).expect("JSON"), error)
}

async fn ok(client: &common::Client, tool: &str, arguments: Value) -> Value {
    let (value, error) = call(client, tool, arguments).await;
    assert!(!error, "{tool}: {value}");
    value
}

async fn save_lane(admin: &common::Client, lane: &str, text: &str) {
    ok(
        admin,
        "set_lane",
        json!({ "lane": lane, "description": format!("The {lane} lane."), "text": text }),
    )
    .await;
}

async fn agent_count(client: &common::Client, token: &str) -> usize {
    ok(client, "list_agents", json!({ "token": token })).await["agents"]
        .as_array()
        .expect("agents")
        .len()
}

#[tokio::test]
async fn agents_take_switch_update_and_drop_lanes_and_their_channels_are_told() {
    let server = start().await;
    let admin = common::connect_admin_with_key(&server, ADMIN_KEY).await;
    let agent = common::connect_new_with_key(&server, ACCESS_KEY).await;
    save_lane(&admin, "reviewer", "Read the diff.").await;
    save_lane(&admin, "writer", "Write the docs.").await;
    save_lane(&admin, "gone", "Old work.").await;
    ok(&admin, "retire_lane", json!({ "lane": "gone" })).await;
    ok(
        &admin,
        "set_server_announcement",
        json!({ "text": "Be brief." }),
    )
    .await;

    // Without a token: the lanes that can be taken, retired ones left out.
    let lanes = ok(&agent, "list_lanes", json!({})).await;
    let names: Vec<_> = lanes["lanes"]
        .as_array()
        .expect("lanes")
        .iter()
        .map(|l| l["lane"].as_str().expect("name").to_string())
        .collect();
    assert_eq!(names, ["reviewer", "writer"]);
    assert_eq!(lanes["lanes"][0]["description"], "The reviewer lane.");
    assert!(lanes["lanes"][0].get("text").is_none(), "{lanes}");

    // Registering with a lane returns it after the server announcement.
    let (text, error) = call_raw(&agent, "register", json!({ "lane": "reviewer" })).await;
    assert!(!error, "{text}");
    let announcement = text
        .find("Server announcement (version 1)")
        .expect("announcement");
    let section = text
        .find("=== Your lane: reviewer (version 1) ===")
        .expect("lane");
    assert!(announcement < section, "{text}");
    assert!(text.contains("A lane grants no permissions."), "{text}");
    assert!(
        text.ends_with("Read the diff.\n=== End of lane reviewer ==="),
        "{text}"
    );
    let a = text
        .lines()
        .find_map(|l| l.strip_prefix("token: "))
        .expect("token")
        .to_string();

    // Another agent shares a channel with it.
    let b = ok(&agent, "register", json!({})).await;
    let b_token = b["token"].as_str().expect("token").to_string();
    let b_handle = b["handle"].as_str().expect("handle").to_string();
    let channel = ok(
        &agent,
        "create_channel",
        json!({ "token": a, "name": "work", "open": false, "members": [b_handle] }),
    )
    .await["id"]
        .as_str()
        .expect("id")
        .to_string();
    let me = ok(&agent, "whoami", json!({ "token": a })).await;
    let handle = me["handle"].as_str().expect("handle").to_string();
    assert_eq!(me["lane"]["name"], "reviewer");
    assert_eq!(me["lane"]["version"], 1);
    assert_eq!(me["lane"]["out_of_date"], false);
    assert_eq!(me["lane"]["text"], "Read the diff.");

    // A newer version: whoami says so, and others and the admin see it marked.
    save_lane(&admin, "reviewer", "Read the diff twice.").await;
    let (whoami, _) = call_raw(&agent, "whoami", json!({ "token": a })).await;
    assert!(
        whoami.contains("A newer version, 2, exists: `take_lane` with lane=`reviewer`"),
        "{whoami}"
    );
    assert!(whoami.contains("Read the diff.\n"), "{whoami}");
    let others = ok(
        &agent,
        "list_agents",
        json!({ "token": b_token, "lane": "reviewer" }),
    )
    .await;
    assert_eq!(others["agents"].as_array().expect("agents").len(), 1);
    assert_eq!(others["agents"][0]["handle"], handle);
    assert_eq!(others["agents"][0]["lane"]["out_of_date"], true);
    assert_eq!(others["agents"][0]["lane"]["current_version"], 2);
    let members = ok(
        &agent,
        "list_members",
        json!({ "token": b_token, "channel_id": channel }),
    )
    .await;
    let member = members["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|m| m["handle"] == handle.as_str())
        .expect("member")
        .clone();
    assert_eq!(member["lane"]["version"], 1);
    let (details, error) = call_raw(&admin, "get_agent", json!({ "handle": handle })).await;
    assert!(!error, "{details}");
    assert!(
        details.contains("lane: reviewer version 1 (out of date: version 2 is the newest)"),
        "{details}"
    );

    // Refusals change nothing and post nothing.
    let before = ok(
        &agent,
        "read",
        json!({ "token": b_token, "channel_id": channel }),
    )
    .await;
    for lane in ["nope", "gone"] {
        let (refused, error) = call(&agent, "take_lane", json!({ "token": a, "lane": lane })).await;
        assert!(error, "{refused}");
        assert_eq!(refused["error"]["code"], "lane_unavailable", "{refused}");
    }
    assert_eq!(
        ok(&agent, "whoami", json!({ "token": a })).await["lane"]["version"],
        1
    );

    // Update, switch, keep and drop, each change told in the channel.
    let steps = [
        (
            Some("reviewer"),
            "updated",
            format!("{handle} moved from version 1 to the reviewer lane (version 2)."),
        ),
        (
            Some("writer"),
            "switched",
            format!("{handle} switched from the reviewer lane to the writer lane (version 1)."),
        ),
        (Some("writer"), "unchanged", String::new()),
        (
            None,
            "dropped",
            format!("{handle} dropped the writer lane."),
        ),
        (None, "unchanged", String::new()),
    ];
    let mut after = before["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|m| m["id"].as_i64())
        .max()
        .unwrap_or(0);
    for (lane, change, told) in steps {
        let taken = ok(&agent, "take_lane", json!({ "token": a, "lane": lane })).await;
        assert_eq!(taken["change"], change, "{taken}");
        let new = ok(
            &agent,
            "read",
            json!({ "token": b_token, "channel_id": channel, "after_id": after }),
        )
        .await;
        let bodies: Vec<_> = new["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .map(|m| m["body"].as_str().expect("body").to_string())
            .collect();
        if told.is_empty() {
            assert!(bodies.is_empty(), "{change}: {bodies:?}");
        } else {
            assert_eq!(bodies, [told], "{change}");
            after = new["messages"][0]["id"].as_i64().expect("id");
        }
    }
    assert!(ok(&agent, "whoami", json!({ "token": a })).await["lane"].is_null());

    // Registering with an unknown or retired lane registers nothing.
    let count = agent_count(&agent, &b_token).await;
    for lane in ["nope", "gone"] {
        let (refused, error) = call(&agent, "register", json!({ "lane": lane })).await;
        assert!(error, "{refused}");
        assert_eq!(refused["error"]["code"], "lane_unavailable", "{refused}");
        assert!(
            refused["error"]["message"]
                .as_str()
                .expect("message")
                .contains("Nothing was registered."),
            "{refused}"
        );
    }
    assert_eq!(agent_count(&agent, &b_token).await, count);
    agent.cancel().await.expect("close");
    admin.cancel().await.expect("close");
}

#[tokio::test]
async fn reclaim_returns_the_lane_held_and_can_take_another() {
    let server = start().await;
    let admin = common::connect_admin_with_key(&server, ADMIN_KEY).await;
    let agent = common::connect_new_with_key(&server, ACCESS_KEY).await;
    save_lane(&admin, "reviewer", "Read the diff.").await;
    save_lane(&admin, "writer", "Write the docs.").await;
    ok(
        &admin,
        "set_server_announcement",
        json!({ "text": "Be brief." }),
    )
    .await;
    let reg = ok(&agent, "register", json!({ "lane": "reviewer" })).await;
    assert_eq!(reg["lane"]["text"], "Read the diff.");
    let handle = reg["handle"].clone();
    let code = reg["recovery_code"].clone();

    // The announcement comes first, then the lane's marked section.
    let in_order = |text: &str, lane: &str, body: &str| {
        let announcement = text
            .find("Server announcement (version 1), saved by the operator (via admin-sentinel):\nBe brief.")
            .unwrap_or_else(|| panic!("announcement: {text}"));
        let section = text
            .find(&format!("=== Your lane: {lane} (version 1) ==="))
            .unwrap_or_else(|| panic!("lane: {text}"));
        assert!(announcement < section, "{text}");
        assert!(
            text.ends_with(&format!("{body}\n=== End of lane {lane} ===")),
            "{text}"
        );
    };

    let kept = ok(
        &agent,
        "reclaim",
        json!({ "handle": handle, "recovery_code": code }),
    )
    .await;
    assert_eq!(kept["lane"]["name"], "reviewer");
    assert_eq!(kept["server_announcement"]["text"], "Be brief.");
    assert_eq!(kept["server_announcement"]["version"], 1);

    // A refused lane leaves the token as it was.
    save_lane(&admin, "gone", "Old work.").await;
    ok(&admin, "retire_lane", json!({ "lane": "gone" })).await;
    for lane in ["nope", "gone"] {
        let (refused, error) = call(
            &agent,
            "reclaim",
            json!({ "handle": handle, "recovery_code": code, "lane": lane }),
        )
        .await;
        assert!(error, "{refused}");
        assert_eq!(refused["error"]["code"], "lane_unavailable", "{refused}");
    }
    let token = kept["token"].as_str().expect("token");
    assert_eq!(
        ok(&agent, "whoami", json!({ "token": token })).await["handle"],
        handle
    );

    let (text, error) = call_raw(
        &agent,
        "reclaim",
        json!({ "handle": handle, "recovery_code": code }),
    )
    .await;
    assert!(!error, "{text}");
    in_order(&text, "reviewer", "Read the diff.");

    let switched = ok(
        &agent,
        "reclaim",
        json!({ "handle": handle, "recovery_code": code, "lane": "writer" }),
    )
    .await;
    assert_eq!(switched["lane"]["name"], "writer");
    assert_eq!(switched["lane"]["text"], "Write the docs.");
    assert_eq!(switched["server_announcement"]["text"], "Be brief.");
    assert_eq!(switched["server_announcement"]["version"], 1);

    let (text, error) = call_raw(
        &agent,
        "reclaim",
        json!({ "handle": handle, "recovery_code": code, "lane": "reviewer" }),
    )
    .await;
    assert!(!error, "{text}");
    in_order(&text, "reviewer", "Read the diff.");
    agent.cancel().await.expect("close");
    admin.cancel().await.expect("close");
}

#[tokio::test]
async fn kkt_register_takes_a_lane_and_refuses_an_unknown_or_retired_one_leaving_no_files() {
    let server = start().await;
    let admin = common::connect_admin_with_key(&server, ADMIN_KEY).await;
    save_lane(&admin, "reviewer", "Read the diff.").await;
    let home = scratch::Dir::new("lanes-register");
    std::fs::write(home.join("key"), ACCESS_KEY).expect("key file");
    let url = format!("{}/mcp", server.base);
    let register = |token_file: &str, lane: &str| {
        let mut cmd = Command::new(KKT);
        cmd.arg("register")
            .args(["--url", &url])
            .arg("--key-file")
            .arg(home.join("key"))
            .arg("--token-file")
            .arg(home.join(token_file))
            .args(["--lane", lane])
            .env("HOME", home.path())
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("KKT_TOKEN")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    };

    let output = register("token", "reviewer").output().await.expect("run");
    assert!(output.status.success(), "{output:?}");
    let token = std::fs::read_to_string(home.join("token")).expect("token");
    let agent = common::connect_new_with_key(&server, ACCESS_KEY).await;
    let me = ok(&agent, "whoami", json!({ "token": token.trim() })).await;
    assert_eq!(me["lane"]["name"], "reviewer");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        me["handle"].as_str().expect("handle")
    );

    let refused = register("other-token", "nope").output().await.expect("run");
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("the server refused the lane: no lane named nope"),
        "{stderr}"
    );
    assert!(!home.join("other-token").exists());

    save_lane(&admin, "gone", "Old work.").await;
    ok(&admin, "retire_lane", json!({ "lane": "gone" })).await;
    let refused = register("other-token", "gone").output().await.expect("run");
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("the server refused the lane: the lane gone is retired"),
        "{stderr}"
    );
    assert!(!home.join("other-token").exists());

    // A refusal for another reason is about the avatar, though its message
    // names the handle `lane-keeper`, which mentions a lane.
    ok(&agent, "register", json!({ "handle": "lane-keeper" })).await;
    let refused = register("other-token", "reviewer")
        .args(["--handle", "lane-keeper"])
        .output()
        .await
        .expect("run");
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("the server refused the avatar request (handle_taken)"),
        "{stderr}"
    );
    assert!(!stderr.contains("refused the lane"), "{stderr}");
    assert!(!home.join("other-token").exists());
    agent.cancel().await.expect("close");
    admin.cancel().await.expect("close");
}
