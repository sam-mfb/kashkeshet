//! Page tests that every implementation of the application must pass,
//! written once over the application as an operator reads it ([`App`]) and
//! a gate and sessions ([`Door`]) that the caller gives, and the helpers
//! for driving the page. kkt-api-sim's tests run them over the simulated
//! application; the server's, over the rules, are in `web_real`'s tests.
//! Nothing here builds a clock, entropy or admission.

use crate::*;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use futures::StreamExt;
use futures::stream::BoxStream;
use kkt_api_agent::{AgentView, Code, Lane, Operator, RuleError, ServerAnnouncement};
use kkt_api_operator::{
    ChannelActivity, ChannelChange, ChannelHidden, ChannelView, LaneHistory, Left, Notice,
    OperatorApi, Overview, ServerRunInfo, StaleAgent, StaleHidden,
};
use kkt_clock::{Span, Time};
use kkt_names as names;
use tower::ServiceExt;

/// Web's own items the server's runs need, which `web` does not export.
pub mod toast {
    pub use crate::toast::{Part, of};
}

/// The activity window the page asks the application for, for tests that
/// call `overview` or `hidden_channels` themselves.
pub const ACTIVITY_WINDOW: std::time::Duration = WEB_ACTIVITY_WINDOW;
/// The activity bucket the page asks the application for, likewise.
pub const ACTIVITY_BUCKET: std::time::Duration = WEB_ACTIVITY_BUCKET;

pub fn start_time() -> Time {
    Time::from_text("2026-10-25T12:00:00Z").expect("valid time")
}

/// The page's size in every scenario but [`earlier_pages_work`]: the
/// default.
const PAGE_MESSAGES: usize = PageLimits::DEFAULT.page_messages;

/// What a site in a scenario is behind: a gate, and the page's sessions,
/// which the caller makes with its own clock and entropy.
pub struct Door<S> {
    pub gate: Arc<dyn Gate<SignedIn = S>>,
    pub sessions: Arc<Sessions<S>>,
}

/// The application a scenario's page shows, and the operator the scenario
/// reads and changes it as, to check what the page did.
#[derive(Clone)]
pub struct App {
    pub api: Arc<dyn OperatorApi>,
    pub operator: Operator,
}

impl App {
    pub fn new(api: Arc<dyn OperatorApi>, operator: Operator) -> App {
        App { api, operator }
    }

    fn actor(&self) -> kkt_api_agent::Actor {
        kkt_api_agent::Actor::Operator(self.operator.clone())
    }

    pub async fn overview(
        &self,
        window: std::time::Duration,
        bucket: std::time::Duration,
    ) -> Result<Overview, RuleError> {
        self.api.overview(&self.operator, window, bucket).await
    }

    pub async fn hidden_channels(
        &self,
        window: std::time::Duration,
        bucket: std::time::Duration,
    ) -> Result<Vec<ChannelActivity>, RuleError> {
        self.api
            .hidden_channels(&self.operator, window, bucket)
            .await
    }

    pub async fn channel(
        &self,
        id: String,
        before_id: Option<i64>,
        page_messages: usize,
    ) -> Result<ChannelView, RuleError> {
        self.api
            .channel_view(&self.operator, id, before_id, page_messages)
            .await
    }

    pub async fn agent(&self, handle: String) -> Result<AgentView, RuleError> {
        self.api.agent_view(&self.operator, handle).await
    }

    pub async fn server_runs(&self) -> Result<Vec<ServerRunInfo>, RuleError> {
        self.api.server_runs(&self.operator).await
    }

    pub async fn change_channel(
        &self,
        channel_id: String,
        change: ChannelChange,
    ) -> Result<(), RuleError> {
        self.api
            .change_channel(self.actor(), channel_id, change)
            .await
    }

    pub async fn hide_agent(&self, handle: String) -> Result<(), RuleError> {
        self.api.hide_agent(self.actor(), handle).await
    }

    pub async fn stale_agents(
        &self,
        idle: std::time::Duration,
    ) -> Result<Vec<StaleAgent>, RuleError> {
        self.api.stale_agents(&self.operator, idle).await
    }

    pub async fn hide_stale_agents(
        &self,
        idle: std::time::Duration,
        chosen: Vec<String>,
    ) -> Result<StaleHidden, RuleError> {
        self.api.hide_stale_agents(self.actor(), idle, chosen).await
    }

    /// The announcement's history, newest first.
    pub async fn server_announcement(&self) -> Result<Vec<ServerAnnouncement>, RuleError> {
        self.api.server_announcement_history(&self.operator).await
    }

    pub async fn lanes(&self) -> Result<Vec<Lane>, RuleError> {
        self.api.lanes(&self.operator).await
    }

    pub async fn lane(&self, name: String) -> Result<LaneHistory, RuleError> {
        self.api.lane_history(&self.operator, name).await
    }

    /// Everything that happens from now on, as the page hears it.
    pub async fn live(&self) -> BoxStream<'static, Notice> {
        let notices = self
            .api
            .subscribe(&self.operator)
            .await
            .expect("the operator may listen");
        futures::stream::unfold(notices, |mut notices| async move {
            let notice = notices.next().await?;
            Some((notice, notices))
        })
        .boxed()
    }
}

/// The site over `source`, behind `door`, with the default settings.
pub fn site<S: Held>(source: App, door: Door<S>) -> Router {
    site_with(source, door, Settings::DEFAULT)
}

/// The site over `source`, behind `door`, with `settings`.
pub fn site_with<S: Held>(source: App, door: Door<S>, settings: Settings) -> Router {
    router(
        source.api,
        AllowedHosts::new(&[]).expect("loopback hosts"),
        door.gate,
        door.sessions,
        Arc::new(kkt_log::testing::CapturedLog::default()),
        settings,
    )
}

pub async fn get(site: &Router, path: &str) -> (StatusCode, String) {
    let request = HttpRequest::builder()
        .uri(path)
        .header("Host", "localhost")
        .body(Body::empty())
        .expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A form post to `path`, sent from this site.
pub async fn post(site: &Router, path: &str, body: &str) -> (StatusCode, Option<String>) {
    let request = HttpRequest::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("Origin", "http://localhost")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|l| l.to_str().ok())
        .map(str::to_string);
    (response.status(), location)
}

/// The value of the first `form_token` field in `page`.
pub fn form_token_in(page: &str) -> String {
    let (_, rest) = page
        .split_once(r#"name="form_token" value=""#)
        .expect("a form token");
    rest.split_once('"').expect("closing quote").0.to_string()
}

/// What every implementation must do for a change: refuse it without the page's
/// form token; with it, make it, and show the operator's message saying so.
pub async fn changes_work<S: Held>(source: App, door: Door<S>) {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    let id = overview
        .channels
        .first()
        .expect("a channel")
        .info
        .id
        .clone();
    let site = site(source.clone(), door);
    let path = format!("/channels/{id}/purpose");
    let (_, page) = get(&site, &format!("/channels/{id}")).await;
    assert!(page.contains(&format!(r#"action="/channels/{id}/purpose""#)));
    let token = form_token_in(&page);

    let (status, _) = post(&site, &path, "purpose=forged").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = post(&site, &path, "purpose=forged&form_token=guess").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let view = source
        .channel(id.clone(), None, PAGE_MESSAGES)
        .await
        .expect("view");
    assert_ne!(view.channel.purpose.as_deref(), Some("forged"));

    let (status, location) = post(
        &site,
        &path,
        &format!("purpose=Release+%26+prep&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, Some(format!("/channels/{id}")));
    let view = source
        .channel(id.clone(), None, PAGE_MESSAGES)
        .await
        .expect("view");
    assert_eq!(view.channel.purpose.as_deref(), Some("Release & prep"));
    let said = view.messages.last().expect("the operator's message");
    assert!(names::is_operator(&said.agent));
    assert_eq!(said.agent, kkt_names::OPERATOR);
    assert_eq!(
        said.body,
        "The operator changed the purpose to: Release & prep"
    );
    // The page shows the operator by name, with no link to an agent page.
    let (_, page) = get(&site, &format!("/channels/{id}")).await;
    assert!(page.contains(r#"<span class="who">operator</span>"#));
    assert!(!page.contains("/agents/operator"));
    // So does the front page, where the operator spoke last in the channel.
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    let summary = overview
        .channels
        .iter()
        .find(|c| c.info.id == id)
        .expect("the channel");
    assert!(
        summary
            .last_speaker
            .as_deref()
            .is_some_and(names::is_operator)
    );
    let (_, page) = get(&site, "/").await;
    assert!(!page.contains("/agents/operator"));

    let (status, _) = post(
        &site,
        "/channels/ch_missing0/purpose",
        &format!("purpose=x&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// What every implementation must do for an agent's handoff: show each of its
/// memory entries, escaped, and give a new recovery code only to a form from
/// the page, on a page the browser does not keep.
pub async fn handoff_works<S: Held>(source: App, door: Door<S>) -> (String, String) {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    let mut noted = None;
    for a in &overview.agents {
        let view = source.agent(a.info.handle.clone()).await.expect("view");
        if view.memory.len() > 1 {
            noted = Some(view);
            break;
        }
    }
    let view = noted.expect("an agent with memory");
    let name = view.agent.handle.clone();
    let site = site(source.clone(), door);
    let (_, page) = get(&site, &format!("/agents/{name}")).await;
    assert!(page.contains(&format!("Memory ({})", view.memory.len())));
    for entry in &view.memory {
        assert!(page.contains(&format!(">{}</h3>", entry.name)), "{page}");
        let first_line = entry.text.lines().next().expect("a line");
        let escaped = first_line.replace('<', "&#60;").replace('>', "&#62;");
        assert!(
            page.contains(first_line) || page.contains(&escaped),
            "the page shows the entry {}",
            entry.name
        );
    }
    assert!(!page.contains("<b>Ask"), "entries are escaped");
    let path = format!("/agents/{name}/recovery-code");
    assert!(page.contains(&format!(r#"action="{path}""#)));
    let token = form_token_in(&page);

    let (status, _) = post(&site, &path, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let request = HttpRequest::builder()
        .method("POST")
        .uri(&path)
        .header("Host", "localhost")
        .header("Origin", "http://localhost")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("form_token={token}")))
        .expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.as_bytes()),
        Some(&b"no-store"[..])
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let page = String::from_utf8_lossy(&bytes).into_owned();
    let (_, rest) = page
        .split_once(r#"<p class="mono" style="overflow-wrap: anywhere">"#)
        .expect("the code");
    let code = rest.split_once('<').expect("end of code").0.to_string();
    assert!(code.starts_with("arc_"), "{code}");

    let (status, _) = post(
        &site,
        "/agents/nobody-here/recovery-code",
        &format!("form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    (name, code)
}

/// What every implementation must do for "Show previous": from the first page of the
/// busiest channel, the earlier pages read back every message once, oldest
/// first, a page at a time, until there are no more. The site shows pages of
/// `settings.page.page_messages`. Returns how many pages there were.
pub async fn earlier_pages_work<S: Held>(source: App, door: Door<S>, settings: Settings) -> usize {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    let busiest = overview
        .channels
        .iter()
        .max_by_key(|c| c.message_count)
        .expect("a channel");
    let id = busiest.info.id.clone();
    let site = site_with(source.clone(), door, settings);
    let first = source
        .channel(id.clone(), None, settings.page.page_messages)
        .await
        .expect("first page");
    let (_, page) = get(&site, &format!("/channels/{id}")).await;
    let before = first.older_before_id.expect("earlier messages to load");
    assert!(page.contains(&format!(r#"data-before="{before}""#)));
    assert!(page.contains(&format!(
        "Show previous {} messages",
        settings.page.page_messages
    )));

    let mut ids: Vec<i64> = first.messages.iter().map(|m| m.id).collect();
    let mut next = Some(before);
    let mut pages = 1;
    while let Some(before) = next {
        let (status, body) = get(
            &site,
            &format!("/channels/{id}/messages?before_id={before}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        let page: Vec<i64> = json["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .map(|m| m["id"].as_i64().expect("id"))
            .collect();
        assert!(!page.is_empty() && page.len() <= settings.page.page_messages);
        assert!(page.iter().all(|&m| m < before), "only earlier messages");
        ids.splice(0..0, page);
        next = json["older_before_id"].as_i64();
        pages += 1;
    }
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(ids, sorted, "oldest first, each once");
    assert_eq!(ids.len() as i64, busiest.message_count, "every message");

    let (status, _) = get(&site, "/channels/ch_missing0/messages?before_id=1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&site, &format!("/channels/{id}/messages")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "before_id is required");
    pages
}

/// What every implementation must do: serve each page, page back through a
/// channel, and answer 404 for what does not exist.
pub async fn pages_work<S: Held>(source: App, door: Door<S>) {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    let channel = overview.channels.first().expect("a channel").info.clone();
    let agent = overview.agents.first().expect("an agent").info.clone();
    let busy = overview
        .channels
        .iter()
        .find(|c| c.message_count > 0)
        .expect("a channel with messages")
        .clone();
    let site = site(source.clone(), door);

    let (status, page) = get(&site, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains(&format!("#{}", channel.name)),
        "front page lacks the channel"
    );
    assert!(page.contains(&agent.handle), "front page lacks the agent");
    assert!(page.contains(&format!("data-now=\"{}\"", overview.now)));
    assert!(
        page.contains(&format!("data-channel-id=\"{}\"", busy.info.id)),
        "front page lacks a channel row"
    );
    assert!(
        !page.contains("<li class=\"msg\""),
        "the front page lists no messages"
    );

    let (status, page) = get(&site, &format!("/channels/{}", channel.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&format!("#{}", channel.name)));
    assert!(page.contains("data-follow"), "the newest page is live");
    assert!(page.contains(&format!("data-max=\"{}\"", PAGE_MESSAGES)));

    let view = source
        .channel(busy.info.id.clone(), None, PAGE_MESSAGES)
        .await
        .expect("channel view");
    let newest = view.messages.last().expect("a message").id;
    let (status, page) = get(
        &site,
        &format!("/channels/{}?before_id={newest}", busy.info.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("Latest messages"),
        "a page of older messages links back"
    );

    let (status, page) = get(&site, &format!("/agents/{}", agent.handle)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&agent.handle));

    // An agent's description shows on the front page, its own page, and as
    // the title of its chip on the pages of channels it is in.
    let described = overview
        .agents
        .iter()
        .find_map(|a| Some((a, a.info.description.clone()?)))
        .expect("an agent with a description");
    let (a, text) = described;
    let (_, page) = get(&site, "/").await;
    assert!(page.contains(&text), "front page lacks a description");
    let (_, page) = get(&site, &format!("/agents/{}", a.info.handle)).await;
    assert!(page.contains(&text), "agent page lacks its description");
    if let Some(c) = a.channels.first() {
        let (_, page) = get(&site, &format!("/channels/{}", c.id)).await;
        assert!(
            page.contains(&format!("title=\"{text}\"")),
            "channel page lacks a member's description"
        );
    }

    let (status, _) = get(&site, "/channels/ch_nothere0").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&site, "/agents/nobody-here").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// What every implementation's channel pages must show in their member lists: each
/// member's lane, and an excerpt of its latest message in the channel that
/// links to a page holding the whole message. Returns how many excerpts
/// were shown, so a caller can tell the check had something to check.
pub async fn member_lists_show_lanes_and_latest_messages<S: Held>(
    source: App,
    door: Door<S>,
) -> usize {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    let site = site(source.clone(), door);
    let mut shown = 0;
    for c in overview.channels.iter().filter(|c| c.message_count > 0) {
        let id = &c.info.id;
        let view = source
            .channel(id.clone(), None, PAGE_MESSAGES)
            .await
            .expect("channel");
        let (status, page) = get(&site, &format!("/channels/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            page.matches("class=\"latest").count(),
            view.latest_messages.len(),
            "one excerpt per member that has sent a message"
        );
        for m in &view.latest_messages {
            assert!(view.members.iter().any(|a| a.handle == m.agent));
            assert!(
                !view
                    .messages
                    .iter()
                    .any(|x| x.agent == m.agent && x.id > m.id),
                "the latest message of {} is its newest",
                m.agent
            );
            let path = format!("/channels/{id}?before_id={}", m.id + 1);
            assert!(
                page.contains(&format!("{path}#m{}", m.id)),
                "no link to message {}",
                m.id
            );
            let (status, linked) = get(&site, &path).await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                linked.contains(&format!("id=\"m{}\"", m.id)),
                "the linked page holds message {}",
                m.id
            );
            shown += 1;
        }
        for a in &view.members {
            if let Some(l) = &a.lane {
                assert!(
                    page.contains(&format!("lane <a href=\"/lanes/{}\">", l.name)),
                    "no lane for {}",
                    a.handle
                );
            }
        }
    }
    assert!(shown > 0, "some member has sent a message");
    shown
}

/// What every implementation must do with hidden channels: leave them off the front
/// page unless asked, list them marked when asked, and show a hidden
/// channel's page to read but not to change.
pub async fn hidden_channels_show_only_on_request<S: Held>(source: App, door: Door<S>) {
    let site = site(source.clone(), door);
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    let id = overview.channels[0].info.id.clone();
    let still_listed = overview.channels[1].info.id.clone();
    source
        .change_channel(id.clone(), ChannelChange::Hide)
        .await
        .expect("hide");
    let hidden = source
        .hidden_channels(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("hidden");
    assert!(hidden.iter().any(|c| c.info.id == id));
    for c in &hidden {
        let view = source
            .channel(c.info.id.clone(), None, PAGE_MESSAGES)
            .await
            .expect("view");
        assert!(view.hidden, "{}", c.info.name);
    }
    assert!(
        source
            .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
            .await
            .expect("overview")
            .channels
            .iter()
            .all(|c| !hidden.iter().any(|h| h.info.id == c.info.id))
    );

    let row = |id: &str| format!("<tr data-channel-id=\"{id}\"");
    let (status, page) = get(&site, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"<a href="/?hidden=1" data-hidden-toggle>Show hidden channels</a>"#));
    assert!(page.contains(&row(&still_listed)));
    for c in &hidden {
        assert!(!page.contains(&row(&c.info.id)), "{}", c.info.name);
    }
    assert!(!page.contains("data-hidden-channel"));

    let (status, page) = get(&site, "/?hidden=1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"<a href="/" data-hidden-toggle>Leave out hidden channels</a>"#));
    assert!(page.contains(&row(&still_listed)));
    for c in &hidden {
        assert!(
            page.contains(&format!("{} data-hidden-channel", row(&c.info.id))),
            "{}",
            c.info.name
        );
    }
    assert_eq!(page.matches("data-hidden-channel").count(), hidden.len());
    assert!(page.contains(&format!("and {} hidden", hidden.len())));
    // Any other value is the default view.
    let (_, page) = get(&site, "/?hidden=yes").await;
    assert!(!page.contains("data-hidden-channel"));

    let (status, page) = get(&site, &format!("/channels/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("data-hidden-notice"));
    assert!(page.contains(r#"<span class="badge gone">hidden</span>"#));
    assert!(page.contains("The operator hid the channel. Its messages are kept."));
    assert!(!page.contains(&format!("/channels/{id}/purpose")));
    assert!(!page.contains(&format!("/channels/{id}/hide")));
    assert!(!page.contains(" data-live data-follow"));
    let (_, page) = get(&site, &format!("/channels/{still_listed}")).await;
    assert!(!page.contains("data-hidden-notice"));
    assert!(page.contains(" data-live data-follow"));

    // A change sent anyway is refused, as for a channel that does not exist.
    let (_, page) = get(&site, "/").await;
    let token = form_token_in(&page);
    let (status, _) = post(
        &site,
        &format!("/channels/{id}/purpose"),
        &format!("purpose=back&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The front page's summary must agree with what the channel and agent
/// pages show, for any implementation.
pub async fn summaries_agree_with_the_pages<S: Held>(source: App, door: Door<S>) {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    for c in &overview.channels {
        let view = source
            .channel(c.info.id.clone(), None, PAGE_MESSAGES)
            .await
            .expect("view");
        let newest = view.messages.last();
        assert_eq!(c.last_message_at, newest.map(|m| m.created_at.clone()));
        assert_eq!(c.last_speaker, newest.map(|m| m.agent.clone()));
        assert!(c.message_count >= view.messages.len() as i64);
        assert_eq!(c.message_count == 0, view.messages.is_empty());
        assert_eq!(c.info.member_count, view.members.len() as i64);
        let starts: Vec<&str> = c.recent_activity.iter().map(|a| a.start.as_str()).collect();
        let mut sorted = starts.clone();
        sorted.sort();
        assert_eq!(sorted, starts, "recent counts are oldest first");
        for a in &c.recent_activity {
            assert_eq!(
                kkt_clock::floor_text(&a.start, WEB_ACTIVITY_BUCKET).as_deref(),
                Some(a.start.as_str()),
                "stretches start on a stretch boundary"
            );
        }
        // With every message on one page, the recent count is exact.
        if view.older_before_id.is_none() {
            let since = kkt_clock::to_text(
                kkt_clock::from_text(&overview.now).expect("now")
                    - Span::from_std(WEB_ACTIVITY_WINDOW).expect("window"),
            );
            let in_window = view
                .messages
                .iter()
                .filter(|m| m.created_at >= since)
                .count();
            let recent: i64 = c.recent_activity.iter().map(|a| a.count).sum();
            assert_eq!(recent, in_window as i64, "{}", c.info.name);
        }
    }
    for a in &overview.agents {
        let view = source.agent(a.info.handle.clone()).await.expect("view");
        let mut theirs: Vec<&str> = view.channels.iter().map(|c| c.id.as_str()).collect();
        let mut listed: Vec<&str> = a.channels.iter().map(|c| c.id.as_str()).collect();
        theirs.sort();
        listed.sort();
        assert_eq!(listed, theirs, "{}", a.info.handle);
    }
    // The front page keeps channels and agents in creation order.
    let (_, page) = get(&site(source, door), "/").await;
    let rows: Vec<&str> = page
        .split("<tr data-channel-id=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    let mut channels = overview.channels.clone();
    channels
        .sort_by(|a, b| (&a.info.created_at, &a.info.id).cmp(&(&b.info.created_at, &b.info.id)));
    let expected: Vec<&str> = channels.iter().map(|c| c.info.id.as_str()).collect();
    assert_eq!(rows, expected);
    let agent_rows: Vec<&str> = page
        .split("<tr data-agent=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    let mut agents = overview.agents.clone();
    agents.sort_by(|a, b| {
        (&a.info.created_at, &a.info.handle).cmp(&(&b.info.created_at, &b.info.handle))
    });
    let expected: Vec<&str> = agents.iter().map(|a| a.info.handle.as_str()).collect();
    assert_eq!(agent_rows, expected);
}

pub async fn administration_works<S: Held>(source: App, door: Door<S>) {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .unwrap();
    let id = overview.channels[0].info.id.clone();
    let current = source
        .channel(id.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    let name = current.members[0].handle.clone();
    let count = current.members.len();
    let app = site(source.clone(), door);
    let (_, page) = get(&app, &format!("/channels/{id}")).await;
    let token = form_token_in(&page);
    let routes = [
        (
            format!("/channels/{id}/visibility"),
            "open=false".to_string(),
        ),
        (
            format!("/channels/{id}/members/add"),
            format!("handle={name}"),
        ),
        (
            format!("/channels/{id}/members/remove"),
            format!("handle={name}"),
        ),
        (format!("/channels/{id}/hide"), String::new()),
        (format!("/agents/{name}/hide"), String::new()),
        (format!("/channels/{id}/owner"), "owner=".to_string()),
    ];
    for (path, fields) in &routes {
        assert_eq!(post(&app, path, fields).await.0, StatusCode::FORBIDDEN);
        assert_eq!(
            post(&app, path, &format!("{fields}&form_token=invalid"))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(get(&app, path).await.0, StatusCode::METHOD_NOT_ALLOWED);
        let request = HttpRequest::builder()
            .method("POST")
            .uri(path)
            .header("Host", "localhost")
            .header("Origin", "http://other.example")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("{fields}&form_token={token}")))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    // The forged requests made no changes.
    assert_eq!(
        source
            .channel(id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap()
            .members
            .len(),
        count
    );
    assert!(source.agent(name.clone()).await.is_ok());
    let open = !current.channel.open;
    assert_eq!(
        post(
            &app,
            &routes[0].0,
            &format!("open={open}&form_token={token}")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        source
            .channel(id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap()
            .channel
            .open,
        open
    );
    assert_eq!(
        post(
            &app,
            &routes[2].0,
            &format!("handle={name}&form_token={token}")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        source
            .channel(id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap()
            .members
            .len(),
        count - 1
    );
    assert_eq!(
        post(
            &app,
            &routes[1].0,
            &format!("handle={name}&form_token={token}")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    let view = source
        .channel(id.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(view.members.len(), count);
    assert!(names::is_operator(&view.messages.last().unwrap().agent));
    assert!(view.messages.last().unwrap().body.contains("added"));
    assert_eq!(view.channel.owner, current.channel.owner);
    // The operator makes the agent the owner; an owner cannot be hidden.
    let set_owner = |owner: &str| format!("owner={owner}&form_token={token}");
    assert_eq!(
        post(&app, &routes[5].0, &set_owner("")).await.0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        source
            .channel(id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap()
            .channel
            .owner,
        None
    );
    assert_eq!(
        post(&app, &routes[5].0, &set_owner(&name)).await.0,
        StatusCode::SEE_OTHER
    );
    let view = source
        .channel(id.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(view.channel.owner.as_deref(), Some(name.as_str()));
    let said = view.messages.last().unwrap();
    assert!(names::is_operator(&said.agent));
    assert_eq!(
        said.body,
        format!("The operator made {name} the owner of the channel.")
    );
    let (_, page) = get(&app, &format!("/channels/{id}")).await;
    assert!(page.contains(&format!("data-owner=\"{name}\"")), "{page}");
    assert_eq!(
        post(&app, &routes[4].0, &format!("form_token={token}"))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert!(source.agent(name.clone()).await.is_ok());
    // Once it owns no channel, it can be hidden.
    for c in source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .unwrap()
        .channels
    {
        let owned = source
            .channel(c.info.id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap()
            .channel
            .owner;
        if owned.as_deref() == Some(name.as_str()) {
            let path = format!("/channels/{}/owner", c.info.id);
            assert_eq!(
                post(&app, &path, &set_owner("")).await.0,
                StatusCode::SEE_OTHER
            );
        }
    }
    let view = source
        .channel(id.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(view.channel.owner, None);
    assert_eq!(
        view.messages.last().unwrap().body,
        format!(
            "The operator removed {name} as the owner of the channel. \
             The channel no longer has an owner."
        )
    );
    assert_eq!(
        post(&app, &routes[4].0, &format!("form_token={token}"))
            .await
            .0,
        StatusCode::SEE_OTHER
    );
    assert!(matches!(
        source.agent(name.clone()).await,
        Err(RuleError {
            code: Code::NotFound,
            ..
        })
    ));
    let view = source
        .channel(id.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert!(!view.members.iter().any(|m| m.handle == name));
    assert!(names::is_operator(&view.messages.last().unwrap().agent));
    assert!(view.messages.last().unwrap().body.contains("hid"));
    assert_eq!(
        post(&app, &routes[3].0, &format!("form_token={token}")).await,
        (StatusCode::SEE_OTHER, Some("/".into()))
    );
    // The human can still read it, marked hidden.
    assert!(
        source
            .channel(id.clone(), None, PAGE_MESSAGES)
            .await
            .unwrap()
            .hidden
    );
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .unwrap();
    assert!(!overview.channels.iter().any(|c| c.info.id == id));
    assert!(!overview.agents.iter().any(|a| a.info.handle == name));
}

/// The operator's removals and hides reach the live feed, saying the
/// operator did them, with the member counts afterwards.
#[expect(
    clippy::disallowed_methods,
    reason = "the live feed has no clock; a short real wait shows nothing more is coming"
)]
pub async fn operator_changes_are_live(source: App) {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .unwrap();
    // Owners cannot be hidden, so the channels are left with none first.
    for c in &overview.channels {
        source
            .change_channel(c.info.id.clone(), ChannelChange::SetOwner(None))
            .await
            .unwrap();
    }
    let channel = overview
        .channels
        .iter()
        .map(|c| &c.info)
        .find(|c| c.member_count >= 2)
        .expect("a channel with two members")
        .clone();
    let view = source
        .channel(channel.id.clone(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    let (removed, hidden) = (
        view.members[0].handle.clone(),
        view.members[1].handle.clone(),
    );
    let hidden_in: Vec<String> = source
        .agent(hidden.clone())
        .await
        .unwrap()
        .channels
        .iter()
        .map(|c| c.id.clone())
        .collect();

    let mut live = source.live().await;
    source
        .change_channel(
            channel.id.clone(),
            ChannelChange::RemoveMember(removed.clone()),
        )
        .await
        .unwrap();
    source.hide_agent(hidden.clone()).await.unwrap();
    source
        .change_channel(channel.id.clone(), ChannelChange::Hide)
        .await
        .unwrap();

    let mut heard = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), live.next()).await
    {
        match event {
            Notice::Message(_) => {}
            other => heard.push(other),
        }
    }
    let op = names::OPERATOR.to_string();
    assert_eq!(heard.len(), 3, "{heard:?}");
    assert_eq!(
        heard[0],
        Notice::Left(Left {
            agent: removed,
            channel_id: channel.id.clone(),
            channel_name: channel.name.clone(),
            member_count: channel.member_count - 1,
            by: op.clone(),
            by_operator: true,
        })
    );
    let Notice::AgentHidden(gone) = &heard[1] else {
        panic!("expected agent_hidden, got {:?}", heard[1]);
    };
    assert_eq!(
        (&gone.agent, &gone.by, gone.by_operator),
        (&hidden, &op, true)
    );
    let mut left: Vec<String> = gone.channels.iter().map(|c| c.channel_id.clone()).collect();
    left.sort();
    let mut expected = hidden_in;
    expected.sort();
    assert_eq!(left, expected);
    // Each count is one less than before, and the first channel had also
    // lost the removed agent.
    for c in &gone.channels {
        let before = overview
            .channels
            .iter()
            .find(|s| s.info.id == c.channel_id)
            .expect("listed before")
            .info
            .member_count;
        let also_removed = i64::from(c.channel_id == channel.id);
        assert_eq!(c.member_count, before - 1 - also_removed, "{c:?}");
    }
    assert_eq!(
        heard[2],
        Notice::ChannelHidden(ChannelHidden {
            channel_id: channel.id,
            channel_name: channel.name,
            by: op,
            by_operator: true,
        })
    );
}

/// The next live event that is not a message.
#[expect(
    clippy::disallowed_methods,
    reason = "bounds a real wait on the live feed, so a missing event fails instead of hanging"
)]
pub async fn next_change(live: &mut futures::stream::BoxStream<'static, Notice>) -> Notice {
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), live.next())
            .await
            .expect("an event in time")
            .expect("the feed is open");
        if !matches!(event, Notice::Message(_)) {
            return event;
        }
    }
}

/// The row of `page` that starts with `start`, up to its end.
pub fn row_of<'a>(page: &'a str, start: &str) -> Option<&'a str> {
    let (_, rest) = page.split_once(start)?;
    Some(rest.split_once("</tr>").map_or(rest, |(row, _)| row))
}

pub fn agent_row<'a>(page: &'a str, name: &str) -> Option<&'a str> {
    row_of(page, &format!("<tr data-agent=\"{name}\""))
}

pub fn channel_row<'a>(page: &'a str, id: &str) -> Option<&'a str> {
    row_of(page, &format!("<tr data-channel-id=\"{id}\""))
}

/// The member count a channel's dashboard row shows.
pub fn members_shown(page: &str, id: &str) -> i64 {
    let row = channel_row(page, id).expect("the channel's row");
    let (_, rest) = row
        .split_once(r#"<td class="num members">"#)
        .expect("a members cell");
    rest.split_once('<')
        .expect("the cell's end")
        .0
        .parse()
        .expect("a number")
}

/// Whether an agent's dashboard row lists the channel.
pub fn lists_channel(page: &str, agent: &str, id: &str) -> bool {
    agent_row(page, agent)
        .expect("the agent's row")
        .contains(&format!("data-channel-id=\"{id}\""))
}

/// The first function named `name` in the page's script, up to its end.
pub fn script_function<'a>(page: &'a str, name: &str) -> &'a str {
    let (_, rest) = page
        .split_once(&format!("  function {name}("))
        .unwrap_or_else(|| panic!("no function {name}"));
    rest.split_once("\n  }\n")
        .unwrap_or_else(|| panic!("no end to {name}"))
        .0
}

/// What every implementation's front page must carry for its script: each event
/// shows its toast and updates the dashboard with it, every kind of toast
/// part can be drawn, and the dashboard has the marks the script looks for.
/// `front` is the front page, `front_with_hidden` the front page showing
/// hidden channels. Removing a toast or dashboard call fails here.
pub fn page_script_works(front: &str, front_with_hidden: &str) {
    let page = front;
    for (event, update) in [
        ("agent", "dashboardAgent"),
        ("channel", "dashboardChannel"),
        ("joined", "dashboardJoined"),
        ("left", "dashboardLeft"),
        ("channel_hidden", "dashboardChannelHidden"),
        ("agent_hidden", "dashboardAgentHidden"),
    ] {
        let (_, rest) = page
            .split_once(&format!("source.addEventListener(\"{event}\", (e) => {{"))
            .unwrap_or_else(|| panic!("no handler for {event}"));
        let handler = rest.split_once("});").expect("handler's end").0;
        assert!(
            handler.contains("const d = JSON.parse(e.data);"),
            "{event}: {handler}"
        );
        assert!(handler.contains("toast(d.toast);"), "{event}: {handler}");
        assert!(
            handler.contains(&format!("{update}(d);")),
            "{event}: {handler}"
        );
    }

    // Every kind of part the server sends has a way to be drawn.
    let drawn = script_function(page, "toastPart");
    let parts = [
        toast::Part::Text { text: "x".into() },
        toast::Part::Agent { handle: "x".into() },
        toast::Part::GoneAgent { handle: "x".into() },
        toast::Part::Name { name: "x".into() },
        toast::Part::Channel {
            id: "x".into(),
            name: "x".into(),
        },
        toast::Part::Badge {
            class: "x".into(),
            text: "x".into(),
        },
    ];
    for part in parts {
        let json = serde_json::to_value(&part).expect("json");
        let kind = json["kind"].as_str().expect("a kind");
        if kind != "text" {
            assert!(drawn.contains(&format!("case \"{kind}\":")), "{kind}");
        }
    }
    assert!(drawn.contains("default: return p.text;"));
    // A gone agent's handle is drawn as plain text: its page is gone.
    assert!(drawn.contains(r#"case "gone_agent": return el("span", "who", p.handle);"#));
    let toast_fn = script_function(page, "toast");
    assert!(toast_fn.contains("spec.parts.map(toastPart)"));
    assert!(toast_fn.contains("hue(spec.hue_of)"));
    assert!(toast_fn.contains("spec.icon"));
    assert!(toast_fn.contains("toasts.append(t)"));

    // What each dashboard update does with its event.
    let left = script_function(page, "dashboardLeft");
    assert!(left.contains("setMembers(l.channel_id, l.member_count)"));
    assert!(left.contains(
        "changeChannels(l.agent, (pairs) => pairs.filter(([id]) => id !== l.channel_id))"
    ));
    let channel_hidden = script_function(page, "dashboardChannelHidden");
    assert!(channel_hidden.contains(r#"dashboard.hasAttribute("data-show-hidden")"#));
    assert!(channel_hidden.contains("setMembers(h.channel_id, 0)"));
    assert!(channel_hidden.contains("row.dataset.hiddenChannel"));
    assert!(channel_hidden.contains("row.remove()"));
    assert!(channel_hidden.contains("pairs.filter(([id]) => id !== h.channel_id)"));
    let agent_hidden = script_function(page, "dashboardAgentHidden");
    assert!(agent_hidden.contains("row.remove()"));
    assert!(agent_hidden.contains("agentRows.delete(h.agent)"));
    assert!(
        agent_hidden
            .contains("h.channels.forEach((c) => setMembers(c.channel_id, c.member_count))")
    );

    // The marks the script looks for are on the page.
    assert!(page.contains(r#"querySelector("td.members")"#));
    assert!(page.contains(r#"<td class="num members">"#));
    assert!(page.contains(r#"querySelector("td.channels")"#));
    assert!(page.contains(r#"<td class="small channels">"#));
    assert!(page.contains(r#"querySelectorAll("a[data-channel-id]")"#));
    assert!(page.contains(r#"" data-channel-id=""#));
    assert!(page.contains(r#"querySelectorAll("tr[data-agent]")"#));
    assert!(page.contains("<tr data-agent=\""));
    assert!(page.contains(r#"querySelectorAll("tr[data-channel-id]")"#));
    assert!(page.contains("<tr data-channel-id=\""));
    assert!(page.contains(r#"<div class="toasts" id="toasts""#));
    assert!(front_with_hidden.contains("data-dashboard data-show-hidden"));
}

pub async fn announcement_editor_works<S: Held>(source: App, door: Door<S>) {
    let site = site(source.clone(), door);
    let (status, page) = get(&site, "/announcement").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("<h1>Server announcement</h1>"), "{page}");
    assert!(page.contains("No versions saved."));
    // The page's address before the rename still leads to it.
    let old = site
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/instructions")
                .header("Host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(old.headers()["location"], "/announcement");
    let token = form_token_in(&page);
    let foreign = HttpRequest::builder()
        .method("POST")
        .uri("/announcement")
        .header("Host", "localhost")
        .header("Origin", "https://evil.example")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("text=forged&form_token={token}")))
        .unwrap();
    assert_eq!(
        site.clone().oneshot(foreign).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    assert_eq!(
        post(&site, "/announcement", "text=forged").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&site, "/announcement", "text=forged&form_token=wrong")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert!(source.server_announcement().await.unwrap().is_empty());
    assert_eq!(
        post(
            &site,
            "/announcement",
            &format!("text=%3Cscript%3E%26%0Ahello&form_token={token}")
        )
        .await,
        (StatusCode::SEE_OTHER, Some("/announcement".into()))
    );
    let first = source.server_announcement().await.unwrap();
    assert_eq!(first[0].version, 1);
    assert_eq!(first[0].text, "<script>&\nhello");
    let (_, page) = get(&site, "/announcement").await;
    assert!(page.contains("&#60;script&#62;&#38;"));
    assert!(!page.contains("<script>&"));
    assert!(page.contains("Version 1"));
    assert_eq!(first[0].saved_by_admin, None);
    assert!(page.contains("by you"), "{page}");
    assert_eq!(
        post(&site, "/announcement", &format!("text=&form_token={token}"))
            .await
            .0,
        StatusCode::SEE_OTHER
    );
    let history = source.server_announcement().await.unwrap();
    assert_eq!(history[0].text, "");
    assert_eq!(history[0].version, 2);
    assert_eq!(history[1], first[0]);
    let too_long = "x".repeat(source.api.limits().max_server_announcement_chars + 1);
    assert_eq!(
        post(
            &site,
            "/announcement",
            &format!("text={too_long}&form_token={token}")
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(source.server_announcement().await.unwrap(), history);
    // A page read cannot make another version.
    get(&site, "/announcement?text=forged").await;
    assert_eq!(source.server_announcement().await.unwrap().len(), 2);
}

/// What every implementation must do on the Lanes pages: list, create, edit with
/// history, refuse bad saves and forged forms, and retire.
pub async fn lanes_pages_work<S: Held>(source: App, door: Door<S>) {
    let site = site(source.clone(), door);
    let before = source.lanes().await.unwrap().len();
    let (status, page) = get(&site, "/lanes").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("<h1>Lanes</h1>"), "{page}");
    assert!(page.contains("grants no permissions"), "{page}");
    let (_, front) = get(&site, "/").await;
    assert!(front.contains(r#"<a href="/lanes">Lanes</a>"#));
    let token = form_token_in(&page);
    let fields = "name=code-reviewer&description=Reviews+pull+requests.&text=%3Cb%3ERead%3C%2Fb%3E+the+diff.";

    // Forged forms change nothing.
    for forged in [fields.to_string(), format!("{fields}&form_token=wrong")] {
        assert_eq!(
            post(&site, "/lanes", &forged).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let foreign = HttpRequest::builder()
        .method("POST")
        .uri("/lanes")
        .header("Host", "localhost")
        .header("Origin", "https://evil.example")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("{fields}&form_token={token}")))
        .unwrap();
    assert_eq!(
        site.clone().oneshot(foreign).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    // Bad lanes are refused.
    for bad in [
        "name=Code+Reviewer&description=d&text=t",
        "name=code-reviewer&description=&text=t",
        "name=code-reviewer&description=two%0Alines&text=t",
        "name=code-reviewer&description=d&text=+",
    ] {
        assert_eq!(
            post(&site, "/lanes", &format!("{bad}&form_token={token}"))
                .await
                .0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    assert_eq!(source.lanes().await.unwrap().len(), before);

    // Creating leads to the lane's page.
    assert_eq!(
        post(&site, "/lanes", &format!("{fields}&form_token={token}")).await,
        (StatusCode::SEE_OTHER, Some("/lanes/code-reviewer".into()))
    );
    let lanes = source.lanes().await.unwrap();
    assert_eq!(lanes.len(), before + 1);
    let created = lanes.iter().find(|l| l.name == "code-reviewer").unwrap();
    assert_eq!(created.current.version, 1);
    assert_eq!(created.current.text, "<b>Read</b> the diff.");
    assert_eq!(created.current.saved_by_admin, None);
    let (_, list) = get(&site, "/lanes").await;
    assert!(list.contains(r#"href="/lanes/code-reviewer""#), "{list}");
    assert!(list.contains("Reviews pull requests."));
    // The form to make a lane comes first, closed, so the list need not be
    // scrolled past to reach it.
    let form = list
        .find("<details class=\"panel\">\n<summary>New lane</summary>\n<form method=\"post\" action=\"/lanes\">")
        .expect("a closed New lane form");
    let first_lane = list.find("<section class=\"panel\">").expect("a lane");
    assert!(form < first_lane, "{list}");
    // Creating it again is refused rather than saved as a new version.
    assert_eq!(
        post(&site, "/lanes", &format!("{fields}&form_token={token}"))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // Editing saves the next version; the page shows both, newest first.
    let (status, page) = get(&site, "/lanes/code-reviewer").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("&#60;b&#62;Read"), "{page}");
    assert!(!page.contains("<b>Read"));
    assert_eq!(
        post(
            &site,
            "/lanes/code-reviewer",
            &format!("description=Reviews+pull+requests.&text=Read+it+twice.&form_token={token}")
        )
        .await,
        (StatusCode::SEE_OTHER, Some("/lanes/code-reviewer".into()))
    );
    let view = source.lane("code-reviewer".into()).await.unwrap();
    assert_eq!(
        view.versions
            .iter()
            .map(|v| (v.version, v.text.as_str()))
            .collect::<Vec<_>>(),
        [(2, "Read it twice."), (1, "<b>Read</b> the diff.")]
    );
    let (_, page) = get(&site, "/lanes/code-reviewer").await;
    let newer = page.find("Version 2 · current").expect(&page);
    let older = page.find("Version 1").expect(&page);
    assert!(newer < older);
    assert!(page.contains("by you"));
    // Editing a lane that doesn't exist does not create it.
    assert_eq!(
        post(
            &site,
            "/lanes/no-such-lane",
            &format!("description=d&text=t&form_token={token}")
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&site, "/lanes/no-such-lane").await.0,
        StatusCode::NOT_FOUND
    );

    // Retiring keeps the lane and its history, and closes the editor.
    assert_eq!(
        post(&site, "/lanes/code-reviewer/retire", "form_token=wrong")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &site,
            "/lanes/code-reviewer/retire",
            &format!("form_token={token}")
        )
        .await,
        (StatusCode::SEE_OTHER, Some("/lanes/code-reviewer".into()))
    );
    let view = source.lane("code-reviewer".into()).await.unwrap();
    assert!(view.lane.retired.is_some());
    assert_eq!(view.versions.len(), 2);
    let (_, page) = get(&site, "/lanes/code-reviewer").await;
    assert!(page.contains("Retired"), "{page}");
    assert!(!page.contains("Save new version"), "{page}");
    assert_eq!(
        post(
            &site,
            "/lanes/code-reviewer",
            &format!("description=d&text=again&form_token={token}")
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (_, list) = get(&site, "/lanes").await;
    assert!(list.contains("· retired"), "{list}");
}

pub async fn channel_creation_works<S: Held>(source: App, door: Door<S>) {
    let before = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .unwrap();
    let member = before.agents[0].info.handle.clone();
    let app = site(source.clone(), door);
    let (_, html) = get(&app, "/").await;
    assert!(html.contains("Create a channel"));
    assert!(html.contains(&format!("name=\"member_{member}\"")));
    let token = form_token_in(&html);
    let fields = format!("name=feedback&purpose=Help+desk&open=false&member_{member}=yes");
    assert_eq!(
        get(&app, "/channels/create").await.0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    for invalid in [fields.clone(), format!("{fields}&form_token=bad")] {
        assert_eq!(
            post(&app, "/channels/create", &invalid).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let forged = HttpRequest::builder()
        .method("POST")
        .uri("/channels/create")
        .header("Host", "localhost")
        .header("Origin", "http://elsewhere.example")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("{fields}&form_token={token}")))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(forged).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    for invalid in [
        "name=closed&open=false",
        "name=&open=true",
        "name=bad%0Aname&open=true",
        "name=unknown&open=false&member_missing-agent=yes",
        "name=test&open=other",
    ] {
        assert_ne!(
            post(
                &app,
                "/channels/create",
                &format!("{invalid}&form_token={token}")
            )
            .await
            .0,
            StatusCode::SEE_OTHER
        );
    }
    assert_eq!(
        source
            .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
            .await
            .unwrap()
            .channels
            .len(),
        before.channels.len()
    );
    let (status, location) = post(
        &app,
        "/channels/create",
        &format!("{fields}&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = location.unwrap();
    let id = location.strip_prefix("/channels/").unwrap();
    let view = source
        .channel(id.into(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert_eq!(view.channel.name, "feedback");
    assert_eq!(view.channel.purpose.as_deref(), Some("Help desk"));
    assert!(!view.channel.open);
    assert_eq!(view.channel.created_by, "operator");
    assert_eq!(view.members.len(), 1);
    assert_eq!(view.members[0].handle, member);
    assert_eq!(view.messages.len(), 1);
    assert!(names::is_operator(&view.messages[0].agent));
    assert_eq!(view.messages[0].body, "The operator created the channel.");
    assert!(view.messages[0].run_id.is_some());
    let (_, html) = get(&app, &location).await;
    assert!(!html.contains("/agents/operator"));
    let (status, location) = post(
        &app,
        "/channels/create",
        &format!("name=public&open=true&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let view = source
        .channel(location.unwrap()[10..].into(), None, PAGE_MESSAGES)
        .await
        .unwrap();
    assert!(view.channel.open);
    assert!(view.members.is_empty());
}

/// A form post to `path`, sent from this site, and the page it answers with.
pub async fn post_for_page(site: &Router, path: &str, body: &str) -> (StatusCode, String) {
    let request = HttpRequest::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("Origin", "http://localhost")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// What every implementation must do for stale agents, given a world where at
/// least one channel owner and two other agents have been quiet for a day:
/// list them with every non-owner selected and owners not selectable, and
/// hide only the agents chosen, never an owner, each as hiding one does.
pub async fn stale_agents_work<S: Held>(source: App, door: Door<S>) {
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    let site = site(source.clone(), door);
    let (_, front) = get(&site, "/").await;
    assert!(front.contains(r#"href="/stale-agents""#));

    let stale = source.stale_agents(day).await.expect("stale");
    let (status, page) = get(&site, "/stale-agents").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"<option value="24" selected>1 day</option>"#));
    let rows: Vec<&str> = page
        .split("<tr data-stale=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    let listed: Vec<&str> = stale.iter().map(|s| s.info.handle.as_str()).collect();
    assert_eq!(rows, listed);
    let (owners, others): (Vec<&StaleAgent>, Vec<&StaleAgent>) =
        stale.iter().partition(|s| !s.owns.is_empty());
    assert!(!owners.is_empty(), "the world has a quiet owner");
    assert!(others.len() >= 2, "the world has two quiet non-owners");
    for s in &others {
        assert!(page.contains(&format!(
            r#"name="agent_{}" value="yes" checked"#,
            s.info.handle
        )));
    }
    for s in &owners {
        assert!(!page.contains(&format!(r#"name="agent_{}""#, s.info.handle)));
        assert!(page.contains(&format!(
            r#"aria-label="{} owns a channel and cannot be hidden""#,
            s.info.handle
        )));
    }
    assert!(page.contains("Owns a channel, so cannot be hidden."));
    assert!(!page.contains("hide it from its own page"));

    let (status, _) = get(&site, "/stale-agents?hours=5").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The operator deselects one and sends the form, with an owner forged in.
    let chosen = &others[0].info.handle;
    let deselected = &others[1].info.handle;
    let owner = &owners[0].info.handle;
    let body = format!("hours=24&agent_{chosen}=yes&agent_{owner}=yes");
    let (status, _) = post_for_page(&site, "/stale-agents/hide", &body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    source.agent(chosen.clone()).await.expect("not hidden yet");

    let token = form_token_in(&page);
    let (status, _) = post_for_page(
        &site,
        "/stale-agents/hide",
        &format!("hours=5&agent_{chosen}=yes&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let was_in = source.agent(chosen.clone()).await.expect("view").channels;
    let (status, after) = post_for_page(
        &site,
        "/stale-agents/hide",
        &format!("{body}&form_token={token}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(after.contains(&format!("Hid 1: {chosen}.")), "{after}");
    assert!(after.contains(&format!("Kept {owner}: it owns a channel.")));
    assert!(!after.contains(&format!(r#"<tr data-stale="{chosen}""#)));
    assert!(after.contains(&format!(r#"<tr data-stale="{deselected}""#)));
    assert!(after.contains(&format!(r#"<tr data-stale="{owner}""#)));
    assert!(matches!(
        source.agent(chosen.clone()).await,
        Err(RuleError {
            code: Code::NotFound,
            ..
        })
    ));
    source
        .agent(deselected.clone())
        .await
        .expect("deselected kept");
    source.agent(owner.clone()).await.expect("owner kept");
    assert!(!was_in.is_empty());
    for c in was_in {
        let view = source
            .channel(c.id.clone(), None, PAGE_MESSAGES)
            .await
            .expect("channel");
        let last = view.messages.last().expect("the operator's message");
        assert!(names::is_operator(&last.agent));
        assert_eq!(
            last.body,
            format!("The operator hid {chosen} and revoked its access. Its messages are kept.")
        );
    }
}

/// What every implementation must do for the upgrade history: the front page shows
/// only this run's version and uptime, with a link to the history; the
/// history lists every run, newest first, with long values wrapped instead
/// of in a table that scrolls sideways.
pub async fn history_works<S: Held>(source: App, door: Door<S>, uptime: &str) {
    let overview = source
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    let current = overview.current_run.expect("the current run");
    let runs = source.server_runs().await.expect("runs");
    assert!(runs.len() >= 2, "an earlier run to show");
    assert_eq!(runs[0], current, "this run is the newest");
    let starts: Vec<&str> = runs.iter().map(|r| r.started_at.as_str()).collect();
    let mut newest_first = starts.clone();
    newest_first.sort_by(|a, b| b.cmp(a));
    assert_eq!(starts, newest_first);
    let site = site(source, door);

    let (status, page) = get(&site, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"<a href="/history">Upgrade history</a>"#));
    assert!(page.contains(&format!("kkt {} · up ", current.version)));
    assert!(
        page.contains(&format!(">{uptime}</span>")),
        "uptime {uptime}"
    );
    for r in &runs[1..] {
        assert!(
            !page.contains(&r.git_commit),
            "the front page lists no earlier runs"
        );
    }

    let (status, page) = get(&site, "/history").await;
    assert_eq!(status, StatusCode::OK);
    let mut at = 0;
    for r in &runs {
        let found = page[at..]
            .find(&format!(r#"data-run="{}""#, r.id))
            .unwrap_or_else(|| panic!("run {} missing or out of order", r.id));
        at += found;
        for long in [&r.git_commit, &r.instructions_sha256] {
            assert!(page.contains(&format!(r#"class="mono wrap">{long}</dd>"#)));
        }
        // The settings hold nothing HTML escapes.
        assert!(page.contains(&format!(r#"<pre class="wrap">{}</pre>"#, r.settings)));
    }
    let (_, main) = page.split_once("<main>").expect("main");
    let (main, _) = main.split_once("</main>").expect("end of main");
    assert!(!main.contains("<table"), "no table to scroll sideways");
    assert!(!main.contains("nowrap"));
    assert!(!main.contains("scroll"));
}
