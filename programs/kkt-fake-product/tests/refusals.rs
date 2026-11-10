//! What the fake product refuses. `Admission::Bearer` lets in any caller
//! with a bearer token, so `FakeIdentify` is what stands between a made-up
//! bearer and a swarm: an unknown bearer, or one used for the wrong role,
//! gets nothing on `/mcp` or the web page.
//!
//! Also one thing the inventory's markers cannot see, because no foreign
//! text appears in it: a handle another swarm holds is refused as taken.
//! That is known and kept: handles are unique per org (one org, in phase
//! 1, holds every swarm), so the refusal says the handle exists in some
//! swarm of the org, and nothing more. The plan lists it, with the
//! server-wide message IDs, whose gaps count other swarms' messages.

use kkt_fake_product::client::Client;
use kkt_fake_product::{Bearers, Product, main, other};
use reqwest::StatusCode;
use serde_json::json;

const UNKNOWN: &str = "a-bearer-nobody-made";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_bearer_gets_nothing() {
    let product = Product::start(
        memory(),
        system_clock(),
        os_entropy(),
        kkt_config::Limits::DEFAULT,
    )
    .await;
    let client = Client::new(&product);
    let registered = client
        .tool(
            Bearers::MAIN_AGENTS,
            "register",
            json!({ "format": "json" }),
        )
        .await
        .json();
    let token = registered["token"].as_str().expect("a token");

    // Admission lets it in; who it is, nobody knows.
    for path in ["/mcp", "/mcp-admin"] {
        assert_eq!(
            client.mcp_status(path, None).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(client.mcp_status(path, Some(UNKNOWN)).await, StatusCode::OK);
    }
    for bearer in [
        UNKNOWN,
        Bearers::MAIN_OPERATOR,
        Bearers::OTHER_OPERATOR,
        Bearers::MAIN_ADMIN,
        Bearers::OTHER_ADMIN,
    ] {
        for (tool, arguments) in [
            ("register", json!({})),
            ("whoami", json!({ "token": token })),
            ("list_channels", json!({ "token": token })),
        ] {
            let answer = client.tool(bearer, tool, arguments).await;
            assert!(answer.is_error, "{bearer} {tool}: {}", answer.text);
            assert!(
                answer.text.contains("unauthenticated"),
                "{bearer} {tool}: {}",
                answer.text
            );
        }
    }
    // The other swarm's agents' bearer does not reach this swarm's avatar.
    let answer = client
        .tool(Bearers::OTHER_AGENTS, "whoami", json!({ "token": token }))
        .await;
    assert!(answer.is_error, "{}", answer.text);

    // At /mcp-admin, only an admin's bearer is an admin.
    for bearer in [UNKNOWN, Bearers::MAIN_AGENTS, Bearers::OTHER_AGENTS] {
        let answer = client.admin(bearer, "list_channels", json!({})).await;
        assert!(answer.is_error, "{bearer}: {}", answer.text);
        assert!(
            answer.text.contains("unauthenticated"),
            "{bearer}: {}",
            answer.text
        );
    }
    let answer = client
        .admin(Bearers::MAIN_ADMIN, "list_channels", json!({}))
        .await;
    assert!(!answer.is_error, "{}", answer.text);

    // Signing in to the web page with it gives a session that shows
    // nothing: the page asks `FakeIdentify` for each request.
    for bearer in [UNKNOWN, Bearers::MAIN_AGENTS] {
        let cookie = client.sign_in(bearer).await;
        for path in ["/", "/lanes", "/history"] {
            let (status, page) = client.page(&cookie, path).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{bearer} {path}: {page}");
            assert!(!page.contains(token), "{page}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handle_taken_in_the_other_swarm_is_taken_and_nothing_more() {
    let product = Product::start(
        memory(),
        system_clock(),
        os_entropy(),
        kkt_config::Limits::DEFAULT,
    )
    .await;
    let client = Client::new(&product);
    let first = client
        .tool(
            Bearers::agents(&main()),
            "register",
            json!({ "handle": "pick-me", "format": "json" }),
        )
        .await;
    assert!(!first.is_error, "{}", first.text);
    let second = client
        .tool(
            Bearers::agents(&other()),
            "register",
            json!({ "handle": "pick-me", "format": "json" }),
        )
        .await;
    assert!(second.is_error, "{}", second.text);
    assert!(second.text.contains("handle_taken"), "{}", second.text);
    let token = first.json()["token"].as_str().expect("a token").to_string();
    for word in [main().as_str(), &token] {
        assert!(!second.raw.contains(word), "{}", second.raw);
    }
    // An unused handle there is free, as anywhere.
    let third = client
        .tool(
            Bearers::agents(&other()),
            "register",
            json!({ "handle": "pick-me-too", "format": "json" }),
        )
        .await;
    assert!(!third.is_error, "{}", third.text);
}

/// A new database in memory, with the core's tables.
fn memory() -> kkt_store::Db {
    kkt_store::Db::new(
        kkt_db_sqlite::MemoryPlace::new()
            .expect("an in-memory database")
            .open(&[kkt_store::schema()])
            .expect("the core's tables"),
    )
}

fn system_clock() -> std::sync::Arc<dyn kkt_clock::Clock> {
    std::sync::Arc::new(kkt_clock::SystemClock)
}

fn os_entropy() -> std::sync::Arc<dyn kkt_names::Entropy> {
    std::sync::Arc::new(kkt_names::OsEntropy)
}
