//! A slow subscriber in one swarm while the other swarm floods. Each
//! swarm's notices have their own buffer in the events bus, so the flood
//! cannot push the slow subscriber's notices out: it gets no `lagged`
//! event, misses nothing, and hears what comes next. Were the buffer
//! shared, the flood would fill it while the slow subscriber's notices
//! wait there, and it would lag, which would also tell it that another
//! swarm is busy.

use kkt_fake_product::client::{Client, SlowFeed};
use kkt_fake_product::{Bearers, Product, main, other};
use kkt_rules::SwarmId;
use serde_json::{Value, json};

/// Each of the waiting swarm's messages is this big, so a few hundred fill
/// the socket and the server's buffers, and the server stops reading the
/// slow subscriber's notices off the bus.
const BIG: usize = 60_000;
/// How many big messages the waiting swarm sends before the flood: enough
/// to fill the socket, far fewer than the bus's buffer holds.
const BIG_MESSAGES: usize = 200;
/// The flood: more notices than one buffer holds.
const FLOOD: usize = kkt_config::EVENT_BUFFER + 200;
/// Agents sending the flood, side by side.
const SENDERS: usize = 16;

async fn agent(client: &Client, swarm: &SwarmId) -> (String, String) {
    let bearer = Bearers::agents(swarm);
    let registered = client
        .tool(bearer, "register", json!({ "format": "json" }))
        .await
        .json();
    let token = registered["token"].as_str().expect("a token").to_string();
    let channel = client
        .tool(
            bearer,
            "create_channel",
            json!({ "token": token, "name": "room", "open": true, "format": "json" }),
        )
        .await
        .json()["id"]
        .as_str()
        .expect("a channel")
        .to_string();
    (token, channel)
}

async fn send(client: &Client, swarm: &SwarmId, token: &str, channel: &str, body: &str) -> Value {
    let answer = client
        .tool(
            Bearers::agents(swarm),
            "send",
            json!({ "token": token, "channel_id": channel, "body": body, "format": "json" }),
        )
        .await;
    assert!(!answer.is_error, "{}", answer.text);
    answer.json()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_subscriber_does_not_lag_while_another_swarm_floods() {
    let limits = kkt_config::Limits {
        max_body_bytes: BIG,
        max_response_bytes: 4 * BIG,
        sends_per_minute: 1_000_000,
        registrations_per_minute: 1_000,
        ..kkt_config::Limits::DEFAULT
    };
    let product = Product::start(memory(), system_clock(), os_entropy(), limits).await;
    let client = Client::new(&product);

    let cookie = client.sign_in(Bearers::operator(&other())).await;
    let mut slow = SlowFeed::open(&product, &cookie).await;

    // The slow subscriber's own swarm fills its socket.
    let (token, channel) = agent(&client, &other()).await;
    let filler = "x".repeat(BIG - 20);
    for i in 0..BIG_MESSAGES {
        send(
            &client,
            &other(),
            &token,
            &channel,
            &format!("big {i} {filler}"),
        )
        .await;
    }

    // The other swarm floods, from several agents at once.
    let mut senders = Vec::new();
    for s in 0..SENDERS {
        let client = Client::new(&product);
        let (token, channel) = agent(&client, &main()).await;
        senders.push(tokio::spawn(async move {
            for i in (s..FLOOD).step_by(SENDERS) {
                send(&client, &main(), &token, &channel, &format!("flood {i}")).await;
            }
        }));
    }
    for sender in senders {
        sender.await.expect("a sender");
    }

    // Its own next message reaches it, after everything before it.
    send(&client, &other(), &token, &channel, "done waiting").await;
    let events = slow.read_until("done waiting").await;
    let names: Vec<&str> = events.iter().map(|e| e.name.as_str()).collect();
    assert!(!names.contains(&"lagged"), "the slow subscriber lagged");
    let messages: Vec<&str> = events
        .iter()
        .filter(|e| e.name == "message")
        .map(|e| e.data.as_str())
        .collect();
    assert_eq!(messages.len(), BIG_MESSAGES + 1, "every message, once");
    for (i, data) in messages[..BIG_MESSAGES].iter().enumerate() {
        assert!(data.contains(&format!("big {i} ")), "message {i} in order");
    }
    assert!(
        events.iter().all(|e| !e.data.contains("flood")),
        "nothing of the flood"
    );
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
    std::sync::Arc::new(kkt_clock_system::SystemClock)
}

fn os_entropy() -> std::sync::Arc<dyn kkt_names::Entropy> {
    std::sync::Arc::new(kkt_names::OsEntropy)
}
