use super::*;
use serde_json::{Value, json};

fn message(id: i64, author: &str, body: Value) -> MessageInput {
    MessageInput {
        id,
        channel_id: "ch_test".into(),
        author: author.into(),
        body: format!("kkt-workboard/1\n{body}"),
        created_at: "2026-10-30T12:00:00Z".into(),
    }
}
fn task(id: i64) -> MessageInput {
    message(
        id,
        "agent-a",
        json!({"request_id":"create","item_id":"agent-a/task","op":"create","kind":"task","payload":{"title":"Migration","state":"queued"}}),
    )
}
fn update(id: i64, base: i64, request: &str) -> MessageInput {
    message(
        id,
        "agent-a",
        json!({"request_id":request,"item_id":"agent-a/task","op":"update","base_message_id":base,"payload":{"title":"Migration","state":"in_progress"}}),
    )
}
fn board(messages: &[MessageInput]) -> Board {
    project(messages, 1793361660, ProjectionLimits::configured()).expect("projection")
}
#[test]
fn ordinary_chat_is_ignored() {
    let mut m = task(1);
    m.body = "example: kkt-workboard/1\n{}".into();
    let b = board(&[m]);
    assert!(b.items.is_empty());
    assert_eq!(b.last_message_id, 1);
}
#[test]
fn first_matching_base_wins() {
    let b = board(&[
        task(142),
        update(143, 142, "start"),
        update(144, 142, "other"),
    ]);
    assert_eq!(b.items["agent-a/task"].revision, 143);
    assert_eq!(b.diagnostics.len(), 1);
}
#[test]
fn retry_preserves_original_revision() {
    let b = board(&[
        task(142),
        update(143, 142, "start"),
        update(144, 142, "start"),
    ]);
    assert_eq!(b.items["agent-a/task"].revision, 143);
    assert!(b.diagnostics.is_empty());
}
#[test]
fn changed_retry_is_diagnostic() {
    let b = board(&[
        task(142),
        update(143, 142, "start"),
        update(144, 143, "start"),
    ]);
    assert_eq!(b.diagnostics.len(), 1);
}
#[test]
fn duplicate_keys_are_rejected() {
    let mut m = task(1);
    m.body = m.body.replace(
        "\"state\":\"queued\"",
        "\"state\":\"queued\",\"state\":\"done\"",
    );
    let b = board(&[m]);
    assert!(b.items.is_empty());
    assert_eq!(b.diagnostics.len(), 1);
}
#[test]
fn malformed_report_does_not_reserve_request_id() {
    let mut bad = task(1);
    bad.body = bad.body.replace("\"queued\"", "\"bogus\"");
    let b = board(&[bad, task(2)]);
    assert_eq!(b.items.len(), 1);
    assert_eq!(b.items["agent-a/task"].revision, 2);
}
#[test]
fn full_and_incremental_replay_match() {
    let a = task(1);
    let b = update(2, 1, "start");
    let full = board(&[a.clone(), b.clone()]);
    let mut incremental = board(&[a]);
    incremental.apply(&b).unwrap();
    assert_eq!(full, incremental);
}
#[test]
fn departed_author_replays() {
    assert_eq!(board(&[task(1)]).items["agent-a/task"].creator, "agent-a");
}
#[test]
fn creation_namespace_and_unknown_fields_are_checked() {
    let mut m = task(1);
    m.author = "other".into();
    assert!(board(&[m]).items.is_empty());
    let mut m = task(1);
    m.body = m.body.replace(
        "\"state\":\"queued\"",
        "\"state\":\"queued\",\"admin\":true",
    );
    assert!(board(&[m]).items.is_empty());
}
#[test]
fn respond_is_unsupported_even_from_operator() {
    let m = message(
        1,
        "operator",
        json!({"request_id":"r","item_id":"agent-a/task","op":"respond","answer":{"outcome":"approved"}}),
    );
    let b = board(&[m]);
    assert!(b.items.is_empty());
    assert_eq!(b.diagnostics.len(), 1);
}
#[test]
fn verify_does_not_advance_revision() {
    let v = message(
        2,
        "agent-b",
        json!({"request_id":"v","item_id":"agent-a/task","op":"verify","item_revision":1,"evidence_message_ids":[1],"success":true}),
    );
    let b = board(&[task(1), v]);
    assert_eq!(b.items["agent-a/task"].revision, 1);
    assert!(b.items["agent-a/task"].verified_at.is_some());
}
#[test]
fn future_or_old_revision_verification_is_not_fresh() {
    for (revision, observed) in [(1, "2099-01-01T00:00:00Z"), (99, "2026-10-30T12:00:00Z")] {
        let v = message(
            2,
            "agent-b",
            json!({"request_id":"v","item_id":"agent-a/task","op":"verify","item_revision":revision,"observed_at":observed,"evidence_message_ids":[1],"success":true}),
        );
        assert!(
            board(&[task(1), v]).items["agent-a/task"]
                .verified_at
                .is_none()
        );
    }
}
fn approval() -> MessageInput {
    message(
        1,
        "agent-a",
        json!({"request_id":"request","item_id":"agent-a/release","op":"create","kind":"approval","payload":{"title":"Release","action":"Publish","scope":"v1","subject":"commit abc","affected_task_ids":[]}}),
    )
}
#[test]
fn resolve_requires_requester_revision_and_evidence() {
    for (author, base, note) in [
        ("agent-b", 1, "done"),
        ("agent-a", 99, "done"),
        ("agent-a", 1, ""),
    ] {
        let r = message(
            2,
            author,
            json!({"request_id":"resolve","item_id":"agent-a/release","op":"resolve","base_message_id":base,"resolution_note":note,"evidence_description":"User answered in session"}),
        );
        assert!(
            board(&[approval(), r]).items["agent-a/release"]
                .resolution
                .is_none()
        );
    }
    let r = message(
        2,
        "agent-a",
        json!({"request_id":"resolve","item_id":"agent-a/release","op":"resolve","base_message_id":1,"resolution_note":"Answered elsewhere","evidence_description":"User session 17"}),
    );
    let b = board(&[approval(), r]);
    assert!(b.items["agent-a/release"].resolution.is_some());
    assert_eq!(b.items["agent-a/release"].status, "reported_resolved");
}
#[test]
fn history_budget_is_explicit() {
    let mut l = ProjectionLimits::configured();
    l.max_messages = 1;
    assert!(project(&[task(1), update(2, 1, "s")], 1793361660, l).is_err());
}
#[test]
fn wrong_channel_and_order_are_rejected() {
    let mut m = update(2, 1, "start");
    m.channel_id = "ch_else".into();
    assert!(project(&[task(1), m], 1793361660, ProjectionLimits::configured()).is_err());
    assert!(
        project(
            &[task(2), update(1, 2, "s")],
            1793361660,
            ProjectionLimits::configured()
        )
        .is_err()
    );
}
#[test]
fn unsafe_reference_urls_and_self_dependencies_are_rejected() {
    let m = message(
        1,
        "agent-a",
        json!({"request_id":"p","item_id":"agent-a/pr","op":"create","kind":"pull_request","payload":{"title":"PR","provider":"github","external_key":"repo/1","url":"javascript:alert(1)","task_ids":[],"status":"open"}}),
    );
    assert!(board(&[m]).items.is_empty());
    let mut m = task(1);
    m.body = m.body.replace(
        "\"state\":\"queued\"",
        "\"state\":\"queued\",\"dependency_ids\":[\"agent-a/task\"]",
    );
    assert!(board(&[m]).items.is_empty());
}
#[test]
fn nested_duplicate_and_unsupported_version_are_diagnosed() {
    let mut m = task(1);
    m.body = m.body.replace("kkt-workboard/1", "kkt-workboard/2");
    assert_eq!(board(&[m]).diagnostics.len(), 1);
    let mut m = task(1);
    m.body = m.body.replace(
        "\"title\":\"Migration\"",
        "\"title\":\"Migration\",\"title\":\"Other\"",
    );
    assert!(board(&[m]).items.is_empty());
}

#[test]
fn item_budget_makes_board_unavailable() {
    let mut limits = ProjectionLimits::configured();
    limits.max_items = 1;
    let mut second = task(2);
    second.body = second
        .body
        .replace("agent-a/task", "agent-a/second")
        .replace("\"request_id\":\"create\"", "\"request_id\":\"second\"");
    assert!(project(&[task(1), second], 1793361660, limits).is_err());
}
#[test]
fn request_content_is_immutable() {
    let r = message(
        2,
        "agent-a",
        json!({"request_id":"change","item_id":"agent-a/release","op":"update","base_message_id":1,"payload":{"title":"Changed","action":"Publish","scope":"v2","subject":"other","affected_task_ids":[]}}),
    );
    let b = board(&[approval(), r]);
    assert_eq!(b.items["agent-a/release"].revision, 1);
    assert_eq!(b.diagnostics.len(), 1);
}
#[test]
fn byte_budget_and_diagnostic_budget_are_bounded() {
    let mut l = ProjectionLimits::configured();
    l.max_bytes = 1;
    assert!(project(&[task(1)], 1793361660, l).is_err());
    let mut l = ProjectionLimits::configured();
    l.max_diagnostics = 1;
    let mut a = task(1);
    a.body = "kkt-workboard/1\n{".into();
    let mut b = a.clone();
    b.id = 2;
    let b = project(&[a, b], 1793361660, l).unwrap();
    assert_eq!(b.diagnostics.len(), 1);
    assert_eq!(b.omitted_diagnostics, 1);
}
#[test]
fn task_dependency_cycle_is_rejected() {
    let a = task(1);
    let b = message(
        2,
        "agent-a",
        json!({"request_id":"b","item_id":"agent-a/b","op":"create","kind":"task","payload":{"title":"B","state":"queued","dependency_ids":["agent-a/task"]}}),
    );
    let u = message(
        3,
        "agent-a",
        json!({"request_id":"cycle","item_id":"agent-a/task","op":"update","base_message_id":1,"payload":{"title":"A","state":"queued","dependency_ids":["agent-a/b"]}}),
    );
    let b = board(&[a, b, u]);
    assert_eq!(b.items["agent-a/task"].revision, 1);
    assert_eq!(b.diagnostics.len(), 1);
}
#[test]
fn expiry_replay_uses_server_message_time() {
    let mut a = approval();
    a.body = a.body.replace(
        "\"scope\":\"v1\"",
        "\"expires_at\":\"2026-10-30T12:01:00Z\",\"scope\":\"v1\"",
    );
    let r = message(
        2,
        "agent-a",
        json!({"request_id":"resolve","item_id":"agent-a/release","op":"resolve","base_message_id":1,"resolution_note":"Answered in session","evidence_description":"Session 4"}),
    );
    let b = project(&[a.clone(), r], i64::MAX, ProjectionLimits::configured()).unwrap();
    assert_eq!(
        b.items["agent-a/release"].status_at(i64::MAX),
        "reported_resolved"
    );
    assert_eq!(
        board(&[a]).items["agent-a/release"].status_at(i64::MAX),
        "expired"
    );
}
#[test]
fn superseding_is_atomic_and_preserves_old_request() {
    let r = message(
        2,
        "agent-a",
        json!({"request_id":"new","item_id":"agent-a/release2","op":"create","kind":"approval","supersedes":"agent-a/release","payload":{"title":"Release 2","action":"Publish","scope":"v2","subject":"new commit","affected_task_ids":[]}}),
    );
    let b = board(&[approval(), r]);
    assert_eq!(b.items.len(), 2);
    assert_eq!(b.items["agent-a/release"].status, "superseded");
}
#[test]
fn deeply_nested_json_is_diagnosed() {
    let mut a = task(1);
    a.body = format!("kkt-workboard/1\n{}0{}", "[".repeat(30), "]".repeat(30));
    let b = board(&[a]);
    assert_eq!(b.diagnostics.len(), 1);
}

#[test]
fn published_v1_fixtures_match_projection() {
    let cases: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/workboard/v1.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let messages: Vec<_> = case["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                message(
                    m["id"].as_i64().unwrap(),
                    m["author"].as_str().unwrap(),
                    m["report"].clone(),
                )
            })
            .collect();
        let b = board(&messages);
        assert_eq!(
            b.diagnostics.len(),
            case["diagnostics"].as_u64().unwrap() as usize,
            "{}",
            case["name"]
        );
        assert_eq!(
            b.items.len(),
            case["revisions"].as_object().unwrap().len(),
            "{}",
            case["name"]
        );
        for (id, revision) in case["revisions"].as_object().unwrap() {
            assert_eq!(
                b.items[id].revision,
                revision.as_i64().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}

#[test]
fn substantive_request_changes_clear_revision_specific_verification() {
    let good = message(
        2,
        "agent-b",
        json!({"request_id":"good","item_id":"agent-a/release","op":"verify","item_revision":1,"evidence_message_ids":[1],"success":true}),
    );
    let bad = message(
        3,
        "agent-b",
        json!({"request_id":"bad","item_id":"agent-a/release","op":"verify","item_revision":1,"evidence_message_ids":[1],"success":false}),
    );
    let operations = [
        json!({"request_id":"withdraw","item_id":"agent-a/release","op":"withdraw","base_message_id":1,"reason":"No longer needed"}),
        json!({"request_id":"resolve","item_id":"agent-a/release","op":"resolve","base_message_id":1,"resolution_note":"Answered","evidence_description":"Session 1"}),
        json!({"request_id":"replace","item_id":"agent-a/release2","op":"create","kind":"approval","supersedes":"agent-a/release","payload":{"title":"New request","action":"Publish","scope":"v2","subject":"def","affected_task_ids":[]}}),
    ];
    for operation in operations {
        let b = board(&[
            approval(),
            good.clone(),
            bad.clone(),
            message(4, "agent-a", operation),
        ]);
        let item = &b.items["agent-a/release"];
        assert_eq!(item.revision, 4);
        assert!(item.verified_at.is_none());
        assert!(item.verification_error.is_none());
    }
}
#[test]
fn verification_and_resolution_keep_navigable_evidence_and_attribution() {
    let v = message(
        2,
        "agent-b",
        json!({"request_id":"verify","item_id":"agent-a/release","op":"verify","item_revision":1,"evidence_message_ids":[1],"success":true}),
    );
    let b = board(&[approval(), v.clone()]);
    let item = &b.items["agent-a/release"];
    assert!(item.evidence_ids().contains(&2));
    assert!(
        item.details()
            .iter()
            .any(|(_, value)| value.contains("agent-b"))
    );
    let r = message(
        3,
        "agent-a",
        json!({"request_id":"resolve","item_id":"agent-a/release","op":"resolve","base_message_id":1,"resolution_note":"Answered","evidence_description":"Session 1","evidence_message_ids":[1,2],"evidence_url":"https://example.com/session/1"}),
    );
    let b = board(&[approval(), v, r]);
    assert!(b.items["agent-a/release"].evidence_ids().contains(&2));
}
