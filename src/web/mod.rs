//! The human's view: pages for all channels, one channel, and one agent,
//! plus a stream of new messages for live updates, and the changes the
//! human makes from them. Requests naming a host that is not allowed are
//! refused, as they are on `/mcp`. When the gate requires it, viewers log in
//! first (`sign_in`). Every change is a form `POST` checked by `change`.
//!
//! The pages read and change only through their own contract (`source`):
//! the server supplies it over the rules (`real`), and the preview from
//! simulated data (`preview`), so the page can run and be worked on without
//! a server.

mod change;
mod notice;
mod preview;
mod real;
mod sign_in;
mod source;

pub use preview::Preview;
pub use real::RulesSource;
pub use source::{
    ActivityCount, Admit, Agent, AgentSummary, AgentView, Channel, ChannelAction, ChannelLink,
    ChannelSummary, ChannelView, Gate, Joined, KeptAgent, KeptReason, Left, LiveEvent, Message,
    NewAgent, NewChannel, OperatorInstructionsVersion, OperatorInstructionsView, Overview,
    ServerRun, SignIn, Source, SourceError, StaleAgent, StaleHidden,
};

use crate::clock;
use crate::config::{
    WEB_ACTIVITY_BUCKET, WEB_ACTIVITY_WINDOW, WEB_KEEP_ALIVE, WEB_STALE_DEFAULT_HOURS,
    WEB_STALE_HOURS,
};
use crate::hosts::AllowedHosts;
use askama::Template;
use axum::Json;
use axum::Router;
use axum::extract::{FromRef, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use change::{Change, NoFields};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;

/// What the pages need: their data, the gate (for form tokens), and
/// whether to offer a logout button.
#[derive(Clone)]
struct Pages {
    source: Arc<dyn Source>,
    gate: Arc<dyn Gate>,
    sign_out: bool,
}

impl FromRef<Pages> for Arc<dyn Gate> {
    fn from_ref(pages: &Pages) -> Arc<dyn Gate> {
        pages.gate.clone()
    }
}

impl Pages {
    /// The token this viewer's forms carry; empty if the viewer has none.
    fn form_token(&self, headers: &HeaderMap) -> String {
        self.gate
            .form_token(sign_in::session_cookie(headers).as_deref())
            .unwrap_or_default()
    }

    fn error(&self, e: SourceError, headers: &HeaderMap) -> Response {
        error_page(e, self.sign_out, self.form_token(headers))
    }
}

/// The web page, reading from `source`. Every route checks the `Host`
/// against `hosts`. When `gate` requires sign-in, every page and the event
/// stream need a session, and `/login` and `/logout` are added.
pub fn router(source: Arc<dyn Source>, hosts: AllowedHosts, gate: Arc<dyn Gate>) -> Router {
    let sign_in_required = gate.sign_in_required();
    let pages = Router::new()
        .route("/", get(index))
        .route("/channels/create", post(create_channel))
        .route("/channels/{id}", get(channel))
        .route("/channels/{id}/messages", get(earlier_messages))
        .route("/channels/{id}/purpose", post(set_purpose))
        .route("/channels/{id}/visibility", post(set_visibility))
        .route("/channels/{id}/members/add", post(add_member))
        .route("/channels/{id}/members/remove", post(remove_member))
        .route("/channels/{id}/owner", post(set_owner))
        .route("/channels/{id}/hide", post(hide_channel))
        .route("/agents/{name}", get(agent))
        .route("/agents/{name}/hide", post(hide_agent))
        .route("/agents/{name}/recovery-code", post(new_recovery_code))
        .route("/stale-agents", get(stale_agents))
        .route("/stale-agents/hide", post(hide_stale_agents))
        .route("/history", get(history))
        .route("/events", get(events))
        .route(
            "/instructions",
            get(operator_instructions).post(save_operator_instructions),
        )
        .with_state(Pages {
            source,
            gate: gate.clone(),
            sign_out: sign_in_required,
        });
    let site = if sign_in_required {
        let sign_in = Router::new()
            .route(
                sign_in::LOGIN_PATH,
                get(sign_in::login_form).post(sign_in::login),
            )
            .route("/logout", post(sign_in::logout))
            .with_state(gate.clone());
        pages
            .route_layer(middleware::from_fn_with_state(
                gate,
                sign_in::require_session,
            ))
            .merge(sign_in)
    } else {
        pages
    };
    site.layer(middleware::from_fn_with_state(hosts, check_host))
}

/// Serves the request only if its `Host` is allowed. Without this, a site
/// whose name was pointed at this machine (DNS rebinding) could read every
/// page from the human's browser.
async fn check_host(State(hosts): State<AllowedHosts>, request: Request, next: Next) -> Response {
    // HTTP/2 carries the host in the URI instead of a `Host` header.
    let host = match request.headers().get(header::HOST) {
        Some(value) => value.to_str().ok().map(str::to_string),
        None => request.uri().authority().map(|a| a.to_string()),
    };
    let allowed = host.as_deref().is_some_and(|h| hosts.allows(h));
    if allowed {
        next.run(request).await
    } else {
        tracing::warn!(host = ?host, "refused a web request naming a host that is not allowed");
        (
            StatusCode::FORBIDDEN,
            "Forbidden: Host header is not allowed",
        )
            .into_response()
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexPage {
    /// In the order they were created; activity never moves them. Hidden
    /// channels follow, when asked for.
    channels: Vec<ChannelSummary>,
    /// Whether hidden channels were asked for (`?hidden=1`).
    show_hidden: bool,
    /// How many of `channels` are hidden.
    hidden_count: usize,
    /// In the order they registered.
    agents: Vec<AgentSummary>,
    /// This server run, if it is recorded.
    current_run: Option<ServerRun>,
    /// How long this server run has been up, as the page says it.
    uptime: Option<String>,
    /// The source's time, for "how long ago".
    now: String,
    /// The stretch of time "recent" messages are counted over, in seconds.
    window_secs: u64,
    /// The stretches they are counted in, in seconds.
    bucket_secs: u64,
    sign_out: bool,
    form_token: String,
}

#[derive(Template)]
#[template(path = "channel.html")]
struct ChannelPage {
    channel: Channel,
    /// Hidden: shown to read, with no forms to change it.
    hidden: bool,
    owner: Option<String>,
    members: Vec<Agent>,
    messages: Vec<Message>,
    older_before_id: Option<i64>,
    /// The page shows the newest messages.
    latest: bool,
    /// Live updates only make sense when showing the newest messages of a
    /// channel that is not hidden.
    live: bool,
    /// The most messages the page keeps as new ones arrive.
    max_messages: usize,
    sign_out: bool,
    form_token: String,
}

#[derive(Template)]
#[template(path = "agent.html")]
struct AgentPage {
    agent: Agent,
    channels: Vec<Channel>,
    message_count: i64,
    handoff_note: String,
    handoff_note_set_at: Option<String>,
    sign_out: bool,
    form_token: String,
}

#[derive(Template)]
#[template(path = "recovery.html")]
struct RecoveryCodePage {
    agent: String,
    code: String,
    sign_out: bool,
    form_token: String,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage {
    status: u16,
    message: String,
    sign_out: bool,
    form_token: String,
}

#[derive(Deserialize)]
struct ChannelParams {
    before_id: Option<i64>,
}

#[derive(Deserialize)]
struct EarlierParams {
    before_id: i64,
}

/// A page of a channel's messages, as the channel page's "Show previous"
/// button fetches them.
#[derive(Serialize)]
struct EarlierMessages {
    /// Oldest first.
    messages: Vec<Message>,
    /// Set when there are earlier ones still: the `before_id` that reads them.
    older_before_id: Option<i64>,
}

/// The messages just before `before_id`, as JSON, for the channel page to
/// add above the ones it shows.
async fn earlier_messages(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    Query(params): Query<EarlierParams>,
) -> Response {
    match pages.source.channel(id, Some(params.before_id)).await {
        Ok(v) => Json(EarlierMessages {
            messages: v.messages,
            older_before_id: v.older_before_id,
        })
        .into_response(),
        Err(SourceError::NotFound(message)) => (StatusCode::NOT_FOUND, message).into_response(),
        Err(SourceError::Invalid(message)) => (StatusCode::BAD_REQUEST, message).into_response(),
        Err(SourceError::Failed(message)) => {
            tracing::error!(error = %message, "could not read earlier messages");
            (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
        }
    }
}

#[derive(Deserialize)]
struct IndexParams {
    /// `1` lists hidden channels too. Anything else, or nothing, leaves
    /// them out.
    hidden: Option<String>,
}

async fn index(
    State(pages): State<Pages>,
    Query(params): Query<IndexParams>,
    headers: HeaderMap,
) -> Response {
    let show_hidden = params.hidden.as_deref() == Some("1");
    let hidden = match show_hidden {
        true => match pages.source.hidden_channels().await {
            Ok(hidden) => hidden,
            Err(e) => return pages.error(e, &headers),
        },
        false => Vec::new(),
    };
    match pages.source.overview().await {
        Ok(o) => page(IndexPage {
            hidden_count: hidden.len(),
            channels: in_creation_order(o.channels, |c| (&c.channel.created_at, &c.channel.id))
                .into_iter()
                .chain(in_creation_order(hidden, |c| {
                    (&c.channel.created_at, &c.channel.id)
                }))
                .collect(),
            show_hidden,
            agents: in_creation_order(o.agents, |a| (&a.agent.created_at, &a.agent.name)),
            uptime: o
                .current_run
                .as_ref()
                .and_then(|r| uptime(&r.started_at, &o.now)),
            current_run: o.current_run,
            now: o.now,
            window_secs: WEB_ACTIVITY_WINDOW.as_secs(),
            bucket_secs: WEB_ACTIVITY_BUCKET.as_secs(),
            sign_out: pages.sign_out,
            form_token: pages.form_token(&headers),
        }),
        Err(e) => pages.error(e, &headers),
    }
}

/// The time from `started` to `now`, both stored times, as the page says it:
/// the two largest units, such as "3 days 4 hours" or "12 minutes". `None`
/// if either time cannot be read. A start after `now` counts as just started.
fn uptime(started: &str, now: &str) -> Option<String> {
    let up = clock::from_text(now)? - clock::from_text(started)?;
    let minutes = up.num_minutes().max(0);
    let (days, hours, minutes) = (minutes / (24 * 60), minutes / 60 % 24, minutes % 60);
    let unit = |n: i64, name: &str| match n {
        1 => format!("1 {name}"),
        _ => format!("{n} {name}s"),
    };
    Some(match (days, hours, minutes) {
        (0, 0, 0) => "less than a minute".to_string(),
        (0, 0, m) => unit(m, "minute"),
        (0, h, 0) => unit(h, "hour"),
        (0, h, m) => format!("{} {}", unit(h, "hour"), unit(m, "minute")),
        (d, 0, _) => unit(d, "day"),
        (d, h, _) => format!("{} {}", unit(d, "day"), unit(h, "hour")),
    })
}

#[derive(Template)]
#[template(path = "history.html")]
struct HistoryPage {
    /// Newest first.
    runs: Vec<ServerRun>,
    sign_out: bool,
    form_token: String,
}

/// The upgrade history: every server run and what produced it.
async fn history(State(pages): State<Pages>, headers: HeaderMap) -> Response {
    match pages.source.server_runs().await {
        Ok(runs) => page(HistoryPage {
            runs,
            sign_out: pages.sign_out,
            form_token: pages.form_token(&headers),
        }),
        Err(e) => pages.error(e, &headers),
    }
}

/// `items` oldest first, by when they were created and then by a name that
/// tells ties apart. The front page keeps this order: rows never move, and
/// new ones are added at the end.
fn in_creation_order<T>(mut items: Vec<T>, key: impl Fn(&T) -> (&String, &String)) -> Vec<T> {
    items.sort_by(|a, b| key(a).cmp(&key(b)));
    items
}

async fn channel(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    Query(params): Query<ChannelParams>,
    headers: HeaderMap,
) -> Response {
    match pages.source.channel(id, params.before_id).await {
        Ok(v) => page(ChannelPage {
            channel: v.channel,
            hidden: v.hidden,
            owner: v.owner,
            members: v.members,
            messages: v.messages,
            older_before_id: v.older_before_id,
            latest: params.before_id.is_none(),
            // A hidden channel gets no new messages.
            live: params.before_id.is_none() && !v.hidden,
            max_messages: pages.source.page_messages(),
            sign_out: pages.sign_out,
            form_token: pages.form_token(&headers),
        }),
        Err(e) => pages.error(e, &headers),
    }
}

#[derive(Deserialize)]
struct CreateChannelForm {
    name: String,
    #[serde(default)]
    purpose: String,
    open: String,
    #[serde(flatten)]
    members: std::collections::HashMap<String, String>,
}

async fn create_channel(
    State(pages): State<Pages>,
    headers: HeaderMap,
    Change(form): Change<CreateChannelForm>,
) -> Response {
    let open = match form.open.as_str() {
        "true" => true,
        "false" => false,
        _ => {
            return pages.error(
                SourceError::Invalid("choose open or closed".into()),
                &headers,
            );
        }
    };
    let members = form
        .members
        .keys()
        .filter_map(|name| name.strip_prefix("member_").map(str::to_string))
        .collect();
    match pages
        .source
        .create_channel(form.name, form.purpose, open, members)
        .await
    {
        Ok(id) => Redirect::to(&format!("/channels/{}", url_path(&id))).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

#[derive(Deserialize)]
struct PurposeForm {
    purpose: String,
}

/// The human changes a channel's purpose, then sees the channel again, with
/// the operator's message saying what changed.
async fn set_purpose(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<PurposeForm>,
) -> Response {
    match pages.source.set_purpose(id.clone(), form.purpose).await {
        Ok(()) => Redirect::to(&format!("/channels/{}", url_path(&id))).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

#[derive(Deserialize)]
struct VisibilityForm {
    open: String,
}

#[derive(Deserialize)]
struct MemberForm {
    name: String,
}

async fn channel_change(
    pages: Pages,
    id: String,
    headers: HeaderMap,
    change: ChannelAction,
) -> Response {
    let hidden = matches!(change, ChannelAction::Hide);
    match pages.source.change_channel(id.clone(), change).await {
        Ok(()) => Redirect::to(
            if hidden {
                "/".to_string()
            } else {
                format!("/channels/{}", url_path(&id))
            }
            .as_str(),
        )
        .into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

async fn set_visibility(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<VisibilityForm>,
) -> Response {
    let open = match form.open.as_str() {
        "true" => true,
        "false" => false,
        _ => {
            return pages.error(
                SourceError::Invalid("open must be true or false".into()),
                &headers,
            );
        }
    };
    channel_change(pages, id, headers, ChannelAction::SetOpen(open)).await
}

async fn add_member(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<MemberForm>,
) -> Response {
    channel_change(pages, id, headers, ChannelAction::AddMember(form.name)).await
}

async fn remove_member(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<MemberForm>,
) -> Response {
    channel_change(pages, id, headers, ChannelAction::RemoveMember(form.name)).await
}

#[derive(Deserialize)]
struct OwnerForm {
    /// The member to make the owner; empty leaves the channel with none.
    owner: String,
}

/// The human passes a channel's ownership to a member, or clears it.
async fn set_owner(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<OwnerForm>,
) -> Response {
    let owner = Some(form.owner.trim().to_string()).filter(|o| !o.is_empty());
    channel_change(pages, id, headers, ChannelAction::SetOwner(owner)).await
}

async fn hide_channel(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(_): Change<change::NoFields>,
) -> Response {
    channel_change(pages, id, headers, ChannelAction::Hide).await
}

async fn hide_agent(
    State(pages): State<Pages>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Change(_): Change<change::NoFields>,
) -> Response {
    match pages.source.hide_agent(name).await {
        Ok(()) => Redirect::to("/").into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

#[derive(Template)]
#[template(path = "stale.html")]
struct StalePage {
    /// The quiet periods offered.
    periods: Vec<Period>,
    /// The one shown.
    hours: u32,
    /// The longest quiet first.
    agents: Vec<StaleAgent>,
    /// What hiding did, when the page follows it.
    outcome: Option<StaleHidden>,
    sign_out: bool,
    form_token: String,
}

/// A quiet period the stale agents page offers.
struct Period {
    hours: u32,
    label: String,
}

impl Period {
    fn all() -> Vec<Period> {
        WEB_STALE_HOURS
            .iter()
            .map(|&hours| Period {
                hours,
                label: period_label(hours),
            })
            .collect()
    }
}

/// `hours` as the page says it: whole days as days, others as hours.
fn period_label(hours: u32) -> String {
    let (count, unit) = match hours % 24 {
        0 => (hours / 24, "day"),
        _ => (hours, "hour"),
    };
    match count {
        1 => format!("1 {unit}"),
        _ => format!("{count} {unit}s"),
    }
}

/// `hours` as a quiet period, if it is one the page offers.
fn stale_period(hours: u32) -> Result<std::time::Duration, SourceError> {
    match WEB_STALE_HOURS.contains(&hours) {
        true => Ok(std::time::Duration::from_secs(u64::from(hours) * 60 * 60)),
        false => Err(SourceError::Invalid(format!(
            "{hours} hours is not one of the periods offered"
        ))),
    }
}

#[derive(Deserialize)]
struct StaleParams {
    hours: Option<u32>,
}

/// The agents quiet for longer than the period chosen, each selected to be
/// hidden except channel owners, which cannot be.
async fn stale_agents(
    State(pages): State<Pages>,
    Query(params): Query<StaleParams>,
    headers: HeaderMap,
) -> Response {
    let hours = params.hours.unwrap_or(WEB_STALE_DEFAULT_HOURS);
    stale_page(&pages, hours, None, &headers).await
}

#[derive(Deserialize)]
struct HideStaleForm {
    hours: String,
    /// `agent_<name>` for each agent chosen.
    #[serde(flatten)]
    chosen: std::collections::HashMap<String, String>,
}

/// The human hides the chosen stale agents, then sees the list again with
/// what was hidden and what was kept.
async fn hide_stale_agents(
    State(pages): State<Pages>,
    headers: HeaderMap,
    Change(form): Change<HideStaleForm>,
) -> Response {
    let Ok(hours) = form.hours.parse::<u32>() else {
        return pages.error(
            SourceError::Invalid("hours must be a whole number".into()),
            &headers,
        );
    };
    let idle = match stale_period(hours) {
        Ok(idle) => idle,
        Err(e) => return pages.error(e, &headers),
    };
    let mut chosen: Vec<String> = form
        .chosen
        .keys()
        .filter_map(|key| key.strip_prefix("agent_").map(str::to_string))
        .collect();
    chosen.sort();
    match pages.source.hide_stale_agents(idle, chosen).await {
        Ok(outcome) => stale_page(&pages, hours, Some(outcome), &headers).await,
        Err(e) => pages.error(e, &headers),
    }
}

async fn stale_page(
    pages: &Pages,
    hours: u32,
    outcome: Option<StaleHidden>,
    headers: &HeaderMap,
) -> Response {
    let idle = match stale_period(hours) {
        Ok(idle) => idle,
        Err(e) => return pages.error(e, headers),
    };
    match pages.source.stale_agents(idle).await {
        Ok(agents) => page(StalePage {
            periods: Period::all(),
            hours,
            agents,
            outcome,
            sign_out: pages.sign_out,
            form_token: pages.form_token(headers),
        }),
        Err(e) => pages.error(e, headers),
    }
}

/// `text` as one URL path segment.
fn url_path(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The human gives an agent a new recovery code. The page that shows it is
/// the only place it ever appears, so the browser is told not to keep it.
async fn new_recovery_code(
    State(pages): State<Pages>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Change(NoFields {}): Change<NoFields>,
) -> Response {
    match pages.source.new_recovery_code(name.clone()).await {
        Ok(code) => (
            [(header::CACHE_CONTROL, "no-store")],
            page(RecoveryCodePage {
                agent: name,
                code,
                sign_out: pages.sign_out,
                form_token: pages.form_token(&headers),
            }),
        )
            .into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

async fn agent(
    State(pages): State<Pages>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    match pages.source.agent(name).await {
        Ok(v) => page(AgentPage {
            agent: v.agent,
            channels: v.channels,
            message_count: v.message_count,
            handoff_note: v.handoff_note,
            handoff_note_set_at: v.handoff_note_set_at,
            sign_out: pages.sign_out,
            form_token: pages.form_token(&headers),
        }),
        Err(e) => pages.error(e, &headers),
    }
}

/// What happens, as server-sent events, each carrying JSON: `message` (a new
/// message), `agent` (an agent registered), `channel` (a channel was
/// created), `joined` (an agent joined or was added to a channel), `left`
/// (an agent left one, or was removed: `by` says who did it),
/// `channel_hidden` (an owner or the operator hid a channel) and
/// `agent_hidden` (an agent unregistered or the operator hid it, with the
/// channels it left). Joins and leaves carry the channel's member count
/// afterwards, and `agent_hidden` each channel's. A `lagged` event means
/// this stream fell behind and missed some; the page reloads.
async fn events(State(pages): State<Pages>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = pages.source.live().map(|event| {
        Ok(match event {
            LiveEvent::Message(message) => json_event("message", &message),
            LiveEvent::Agent(agent) => json_event("agent", &agent),
            LiveEvent::Channel(channel) => json_event("channel", &channel),
            LiveEvent::Joined(joined) => json_event("joined", &joined),
            LiveEvent::Left(left) => json_event("left", &left),
            LiveEvent::ChannelHidden(hidden) => json_event("channel_hidden", &hidden),
            LiveEvent::AgentHidden(hidden) => json_event("agent_hidden", &hidden),
            LiveEvent::Missed(n) => Event::default().event("lagged").data(n.to_string()),
        })
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(WEB_KEEP_ALIVE).text("keep-alive"))
}

/// An event named `name` carrying `data` as JSON. If that fails, which it
/// should not, the page is told to reload instead.
fn json_event(name: &str, data: &impl serde::Serialize) -> Event {
    match Event::default().event(name).json_data(data) {
        Ok(event) => event,
        Err(e) => {
            tracing::error!(error = %e, "could not encode {name} event");
            Event::default().event("lagged").data("encode error")
        }
    }
}

mod filters {
    /// A stable hue (0 to 359) for an agent's name, so each agent has its own
    /// color. FNV-1a over the name's UTF-8 bytes; the page's script computes
    /// the same for live messages.
    #[askama::filter_fn]
    pub fn hue<T: std::fmt::Display>(name: T, _: &dyn askama::Values) -> askama::Result<u32> {
        Ok(name_hue(&name.to_string()))
    }

    pub fn name_hue(name: &str) -> u32 {
        let mut hash: u32 = 0x811c_9dc5;
        for byte in name.bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
        hash % 360
    }

    /// A name's first character, capitalized, for its avatar.
    #[askama::filter_fn]
    pub fn initial<T: std::fmt::Display>(
        name: T,
        _: &dyn askama::Values,
    ) -> askama::Result<String> {
        Ok(name
            .to_string()
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default())
    }

    /// A stored time, `2026-10-23T14:02:11Z`, shown as `2026-10-23 14:02:11`.
    #[askama::filter_fn]
    pub fn when<T: std::fmt::Display>(stored: T, _: &dyn askama::Values) -> askama::Result<String> {
        Ok(stored.to_string().replace('T', " ").replace('Z', ""))
    }
}

fn page(t: impl Template) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "could not render page");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not render page").into_response()
        }
    }
}

fn error_page(e: SourceError, sign_out: bool, form_token: String) -> Response {
    let (status, message) = match e {
        SourceError::NotFound(message) => (StatusCode::NOT_FOUND, message),
        SourceError::Invalid(message) => (StatusCode::BAD_REQUEST, message),
        SourceError::Failed(message) => {
            tracing::error!(error = %message, "page failed");
            (StatusCode::INTERNAL_SERVER_ERROR, message)
        }
    };
    let body = ErrorPage {
        status: status.as_u16(),
        message,
        sign_out,
        form_token,
    };
    match body.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(render_error) => {
            tracing::error!(error = %render_error, "could not render error page");
            (status, "error").into_response()
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod filter_tests {
    use super::filters;

    #[test]
    fn hues_match_the_page_script() {
        // Values printed by base.html's `hue` function under node 22.
        for (name, hue) in [
            ("amber-falcon", 282),
            ("bold-panther", 299),
            ("swift-jackal", 293),
            ("sunny-heron", 355),
            ("", 61),
            ("é-ünïcode", 304),
        ] {
            assert_eq!(filters::name_hue(name), hue, "{name:?}");
        }
    }

    #[test]
    fn hues_are_stable_and_spread() {
        // The page's script must give the same values (checked with node).
        assert_eq!(
            filters::name_hue("amber-falcon"),
            filters::name_hue("amber-falcon")
        );
        assert_eq!(filters::name_hue(""), 0x811c_9dc5 % 360);
        let hues: std::collections::BTreeSet<u32> = [
            "amber-falcon",
            "bold-panther",
            "swift-jackal",
            "sunny-heron",
        ]
        .iter()
        .map(|n| filters::name_hue(n))
        .collect();
        assert!(hues.iter().all(|&h| h < 360));
        assert_eq!(hues.len(), 4);
    }
}

#[derive(Template)]
#[template(path = "instructions.html")]
struct OperatorInstructionsPage {
    history: Vec<source::OperatorInstructionsVersion>,
    current_text: String,
    max_chars: usize,
    sign_out: bool,
    form_token: String,
}
async fn operator_instructions(State(pages): State<Pages>, headers: HeaderMap) -> Response {
    match pages.source.operator_instructions().await {
        Ok(view) => page(OperatorInstructionsPage {
            current_text: view
                .history
                .first()
                .map_or_else(String::new, |v| v.text.clone()),
            history: view.history,
            max_chars: view.max_chars,
            sign_out: pages.sign_out,
            form_token: pages.form_token(&headers),
        }),
        Err(e) => pages.error(e, &headers),
    }
}
#[derive(Deserialize)]
struct InstructionsForm {
    text: String,
}
async fn save_operator_instructions(
    State(pages): State<Pages>,
    headers: HeaderMap,
    Change(form): Change<InstructionsForm>,
) -> Response {
    match pages.source.save_operator_instructions(form.text).await {
        Ok(()) => Redirect::to("/instructions").into_response(),
        Err(e) => pages.error(e, &headers),
    }
}
