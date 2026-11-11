//! The MCP tools are a contract with every agent that uses the server. This
//! test compares the tools the server offers (names, descriptions, and input
//! schemas) with the snapshot in `tests/contract/tools.json`, so any change to
//! what agents see shows up as a failure.
//!
//! After an intended change, rewrite the snapshot with
//! `UPDATE_CONTRACT=1 cargo test --test contract` and review the diff.

mod common;

use kkt_settings::Limits;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

const UPDATE_VAR: &str = "UPDATE_CONTRACT";

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/contract/tools.json")
}

#[tokio::test]
async fn tools_match_the_snapshot() {
    let server = common::start().await;
    let client = common::connect_new(&server).await;
    let mut tools = client.list_all_tools().await.expect("list tools");
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    let actual = serde_json::to_value(&tools).expect("tools as JSON");
    let path = snapshot_path();

    if std::env::var_os(UPDATE_VAR).is_some() {
        let text = serde_json::to_string_pretty(&actual).expect("pretty JSON");
        std::fs::write(&path, text + "\n").expect("write snapshot");
        return;
    }

    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "could not read {}: {e}. Create it with {UPDATE_VAR}=1",
            path.display()
        )
    });
    let expected: Value = serde_json::from_str(&text).expect("snapshot is JSON");
    assert!(
        actual == expected,
        "the MCP tools changed. If that was intended, run \
         `{UPDATE_VAR}=1 cargo test --test contract` and review the diff.\nactual:\n{}",
        serde_json::to_string_pretty(&actual).expect("pretty JSON")
    );
}

fn admin_snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/contract/admin-tools.json")
}

/// The admin endpoint's tools are a contract with admin agents, pinned the
/// same way in `tests/contract/admin-tools.json`.
#[tokio::test]
async fn admin_tools_match_the_snapshot() {
    let admin_key = "admin-0123456789abcdef0123456789abcdef";
    let mut config = common::test_config();
    config.admin_key = Some(admin_key.into());
    // An admin key with no access key: settings only this process can have.
    let server = common::start_in_process(config).await;
    let client = common::connect_admin_with_key(&server, admin_key).await;
    let mut tools = client.list_all_tools().await.expect("list tools");
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    let actual = serde_json::to_value(&tools).expect("tools as JSON");
    let path = admin_snapshot_path();

    if std::env::var_os(UPDATE_VAR).is_some() {
        let text = serde_json::to_string_pretty(&actual).expect("pretty JSON");
        std::fs::write(&path, text + "\n").expect("write snapshot");
        return;
    }

    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "could not read {}: {e}. Create it with {UPDATE_VAR}=1",
            path.display()
        )
    });
    let expected: Value = serde_json::from_str(&text).expect("snapshot is JSON");
    assert!(
        actual == expected,
        "the admin MCP tools changed. If that was intended, run \
         `{UPDATE_VAR}=1 cargo test --test contract` and review the diff.\nactual:\n{}",
        serde_json::to_string_pretty(&actual).expect("pretty JSON")
    );
}

/// The limits in tool descriptions come from the server's settings, not from
/// fixed text, so a server started with other limits states its own.
#[tokio::test]
async fn descriptions_state_the_live_limits() {
    let mut config = common::test_config();
    config.limits = Limits {
        default_limit: 7,
        max_limit: 42,
        max_body_bytes: 3000,
        max_channel_name_chars: 33,
        max_purpose_chars: 77,
        max_description_chars: 55,
        max_note_chars: 1234,
        max_memory_entry_chars: 4321,
        max_memory_chars: 54321,
        default_wait: Duration::from_secs(5),
        max_wait: Duration::from_secs(9),
        ..Limits::DEFAULT
    };
    let server = common::start_with(config).await;
    let client = common::connect_new(&server).await;
    let tools = client.list_all_tools().await.expect("list tools");
    let param = |tool: &str, name: &str| -> String {
        let tool = tools.iter().find(|t| t.name == tool).expect(tool);
        tool.input_schema["properties"][name]["description"]
            .as_str()
            .expect(name)
            .to_string()
    };

    assert_eq!(
        param("read", "limit"),
        "How many to return (default 7, up to 42)."
    );
    assert_eq!(
        param("read", "max_body_chars"),
        "Cut bodies longer than this many characters. Default: no cut beyond the response size limit."
    );
    assert_eq!(
        param("set_note", "text"),
        "The new pinned note, up to 1234 characters. Empty clears it."
    );
    assert_eq!(
        param("remember", "text"),
        "The entry's text, up to 4321 characters. It may not be empty; `forget` deletes an entry."
    );
    let remember = tools
        .iter()
        .find(|t| t.name == "remember")
        .expect("remember");
    let description = remember.description.as_deref().expect("description");
    assert!(
        description
            .contains("at most 4321 characters, and all your entries together at most 54321"),
        "{description}"
    );
    assert_eq!(param("send", "body"), "The message text, up to 3000 bytes.");
    assert_eq!(
        param("create_channel", "name"),
        "A label for the channel, up to 33 characters. Need not be unique."
    );
    assert_eq!(
        param("set_purpose", "purpose"),
        "The new purpose, up to 77 characters. Empty clears it."
    );
    assert_eq!(
        param("set_description", "text"),
        "The new description, up to 55 characters. Empty clears it."
    );
    assert_eq!(
        param("wait_for_messages", "timeout_s"),
        "Seconds to wait (default 5, up to 9). 0 checks without waiting."
    );
}

/// The error codes are part of the contract: each one's text is fixed. A new
/// code must be added here, as the match lists every code.
#[test]
fn error_codes_are_fixed() {
    use kkt_rules::Code;
    let text = |code: Code| match code {
        Code::Unauthenticated
        | Code::NotFound
        | Code::NotMember
        | Code::BodyTooLong
        | Code::ContainsSecret
        | Code::RateLimited
        | Code::LimitReached
        | Code::InvalidArgument
        | Code::Internal
        | Code::NotOwner
        | Code::HandleTaken
        | Code::LaneUnavailable
        | Code::OperatorOnly => code.as_str(),
    };
    let codes = [
        (Code::Unauthenticated, "unauthenticated"),
        (Code::NotFound, "not_found"),
        (Code::NotMember, "not_member"),
        (Code::BodyTooLong, "body_too_long"),
        (Code::ContainsSecret, "contains_secret"),
        (Code::RateLimited, "rate_limited"),
        (Code::LimitReached, "limit_reached"),
        (Code::InvalidArgument, "invalid_argument"),
        (Code::Internal, "internal"),
        (Code::NotOwner, "not_owner"),
        (Code::HandleTaken, "handle_taken"),
        (Code::LaneUnavailable, "lane_unavailable"),
        (Code::OperatorOnly, "operator_only"),
    ];
    for (code, expected) in codes {
        assert_eq!(text(code), expected, "{code:?}");
    }
}

/// `kkt setup` lets OpenCode use kkt's tools by naming each one, so its
/// list must be the tools the server offers. Adding a tool changes the
/// snapshot above; add it to `KKT_AGENT_TOOLS` too.
#[test]
fn the_tools_setup_allows_are_the_snapshots_tools() {
    let text = std::fs::read_to_string(snapshot_path()).expect("read snapshot");
    let snapshot: Value = serde_json::from_str(&text).expect("snapshot is JSON");
    let mut offered: Vec<&str> = snapshot
        .as_array()
        .expect("a list of tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("a name"))
        .collect();
    offered.sort_unstable();
    let mut allowed = kkt_client_config::KKT_AGENT_TOOLS.to_vec();
    allowed.sort_unstable();
    assert_eq!(allowed, offered);
}
