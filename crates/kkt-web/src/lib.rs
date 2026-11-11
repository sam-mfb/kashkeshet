//! The human's view: pages for all channels, one channel, and one agent,
//! plus a stream of new messages for live updates, and the changes the
//! human makes from them. Requests naming a host that is not allowed are
//! refused, as they are on `/mcp`. The gate decides who is viewing each
//! request (`viewing`), and each viewer reads its own swarm; when the page
//! requires it, viewers log in first (`sign_in`) and hold a session
//! (`sessions`). Every change is a form
//! `POST` checked by `change`.
//!
//! The pages read and change through the application's operations, the
//! kkt-api crates' (`OperatorApi`), in the viewer's swarm, and render their
//! records. Its own contract is only the gate (`gate`): the server supplies
//! the operations over the rules and the gate over admission (`web_real`),
//! and the preview both from simulated data (`preview`), so the page can
//! run and be worked on without a server.

mod change;
mod gate;
mod lanes;
mod preview;
mod sessions;
mod settings;
mod sign_in;
mod toast;
mod viewing;

pub use gate::{Admit, Gate, SignIn, Viewer};
pub use preview::{
    PREVIEW_BIND, PREVIEW_SEED, PREVIEW_SESSION_LIFETIME, Preview, PreviewGate, PreviewSettings,
};
pub use sessions::{Access, Held, Session, Sessions, WEB_SESSION_ID_CHARS};
pub use settings::{MAX_WEB_PAGE_MESSAGES, PageLimits, SettingError, Settings};

use askama::Template;
use axum::Json;
use axum::Router;
use axum::extract::{FromRef, FromRequestParts, Path, Query, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use change::{Change, NoFields};
use futures::{Stream, StreamExt};
use kkt_api_agent::{
    Actor, AgentInfo, ChannelInfo, Code, Message, NewChannel, Operator, Refusal, RuleError,
    ServerAnnouncement,
};
use kkt_api_operator::{
    AgentActivity, ChannelActivity, ChannelChange, KeptReason, Notice, Notices, OperatorApi,
    ServerRunInfo, StaleAgent, StaleHidden,
};
use kkt_clock as clock;
use kkt_hosts::AllowedHosts;
use kkt_log::{Field, Level, Log};
use kkt_names as names;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use viewing::{Forms, GateForms, ViewerApi, Viewing};

/// The front page counts each channel's messages in this last stretch of
/// time, to show which channels are busy. The page gives it to the
/// application.
const WEB_ACTIVITY_WINDOW: Duration = Duration::from_secs(5 * 60);
/// Those messages are counted in stretches this long, so the page gets a few
/// numbers per channel rather than a time for every message, and a count
/// leaves the window at most this late. The page gives it to the
/// application.
const WEB_ACTIVITY_BUCKET: Duration = Duration::from_secs(10);
/// The quiet periods, in hours, the web page offers for finding stale
/// agents: no call for longer than this and no open wait.
const WEB_STALE_HOURS: &[u32] = &[1, 2, 6, 24, 72, 168];
/// The one of them chosen when the page is first opened.
const WEB_STALE_DEFAULT_HOURS: u32 = 24;

/// What every page shares: the gate's forms (for form tokens), whether to
/// offer a logout button, and the page's settings.
#[derive(Clone)]
struct Site {
    gate: Arc<dyn Forms>,
    log: Arc<dyn Log>,
    sign_out: bool,
    settings: Settings,
}

impl FromRef<Site> for Arc<dyn Log> {
    fn from_ref(site: &Site) -> Arc<dyn Log> {
        site.log.clone()
    }
}

impl FromRef<Site> for Arc<dyn Forms> {
    fn from_ref(site: &Site) -> Arc<dyn Forms> {
        site.gate.clone()
    }
}

/// What one request's page needs: the application in the swarm of the
/// viewer the gate admitted, and the viewer's operator
/// (`viewing::admit`), the gate's forms, whether to offer a logout button,
/// and the page's settings.
struct Pages {
    api: Arc<dyn OperatorApi>,
    operator: Operator,
    gate: Arc<dyn Forms>,
    log: Arc<dyn Log>,
    sign_out: bool,
    settings: Settings,
}

impl FromRequestParts<Site> for Pages {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, site: &Site) -> Result<Pages, Response> {
        match parts.extensions.get::<ViewerApi>() {
            Some(ViewerApi { api, operator }) => Ok(Pages {
                api: api.clone(),
                operator: operator.clone(),
                gate: site.gate.clone(),
                log: site.log.clone(),
                sign_out: site.sign_out,
                settings: site.settings,
            }),
            // Every page route is behind `viewing::admit`, so this is a
            // route added without it.
            None => {
                site.log.record(
                    Level::Error,
                    module_path!(),
                    format_args!("a page was served without asking the gate"),
                    &[Field::display("path", &parts.uri.path())],
                );
                Err((StatusCode::INTERNAL_SERVER_ERROR, "no viewer for this page").into_response())
            }
        }
    }
}

impl Pages {
    /// The token this viewer's forms carry; empty if the viewer has none.
    fn form_token(&self, headers: &HeaderMap) -> String {
        self.gate
            .form_token(sign_in::session_cookie(headers).as_deref())
            .unwrap_or_default()
    }

    /// Who makes a change on the page: the viewer's operator.
    fn actor(&self) -> Actor {
        Actor::Operator(self.operator.clone())
    }

    fn error(&self, e: RuleError, headers: &HeaderMap) -> Response {
        error_page(
            self.log.as_ref(),
            e,
            self.sign_out,
            self.form_token(headers),
        )
    }
}

/// The web page. Every route checks the `Host` against `hosts`. `gate` is
/// asked who is viewing every page and the event stream, given the session
/// the request presents, and each viewer reads and changes `api` in the
/// viewer's swarm, as the viewer's operator. When `sessions` require sign-in, a viewer the gate does not admit is
/// sent to `/login`, and `/login` and `/logout` are added; a key the gate
/// accepts there starts a session in `sessions`. The pages are sized by
/// `settings`.
pub fn router<S: Held>(
    api: Arc<dyn OperatorApi>,
    hosts: AllowedHosts,
    gate: Arc<dyn Gate<SignedIn = S>>,
    sessions: Arc<Sessions<S>>,
    log: Arc<dyn Log>,
    settings: Settings,
) -> Router {
    let sign_in_required = sessions.sign_in_required();
    let forms: Arc<dyn Forms> = Arc::new(GateForms {
        gate: gate.clone(),
        sessions: sessions.clone(),
    });
    let viewing = Arc::new(Viewing {
        gate,
        sessions,
        api,
    });
    let state = Site {
        gate: forms.clone(),
        log: log.clone(),
        sign_out: sign_in_required,
        settings,
    };
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
        .route("/agents/{handle}", get(agent))
        .route("/agents/{handle}/hide", post(hide_agent))
        .route("/agents/{handle}/recovery-code", post(new_recovery_code))
        .route("/stale-agents", get(stale_agents))
        .route("/stale-agents/hide", post(hide_stale_agents))
        .route("/history", get(history))
        .route(sign_in::EVENTS_PATH, get(events))
        .route(
            ANNOUNCEMENT_PATH,
            get(server_announcement).post(save_server_announcement),
        )
        // The page's address before the rename, kept for bookmarks.
        .route("/instructions", get(old_announcement_address))
        .route(
            lanes::LANES_PATH,
            get(lanes::lanes).post(lanes::create_lane),
        )
        .route("/lanes/{name}", get(lanes::lane).post(lanes::save_lane))
        .route("/lanes/{name}/retire", post(lanes::retire_lane))
        .route_layer(middleware::from_fn_with_state(viewing, viewing::admit::<S>))
        .with_state(state.clone());
    let site = if sign_in_required {
        let sign_in = Router::new()
            .route(
                sign_in::LOGIN_PATH,
                get(sign_in::login_form).post(sign_in::login),
            )
            .route("/logout", post(sign_in::logout))
            .with_state(state);
        pages.merge(sign_in)
    } else {
        pages
    };
    site.layer(middleware::from_fn_with_state(
        HostGate { hosts, log },
        check_host,
    ))
}

#[derive(Clone)]
struct HostGate {
    hosts: AllowedHosts,
    log: Arc<dyn Log>,
}

/// Serves the request only if its `Host` is allowed. Without this, a site
/// whose name was pointed at this machine (DNS rebinding) could read every
/// page from the human's browser.
async fn check_host(State(state): State<HostGate>, request: Request, next: Next) -> Response {
    // HTTP/2 carries the host in the URI instead of a `Host` header.
    let host = match request.headers().get(header::HOST) {
        Some(value) => value.to_str().ok().map(str::to_string),
        None => request.uri().authority().map(|a| a.to_string()),
    };
    let allowed = host.as_deref().is_some_and(|h| state.hosts.allows(h));
    if allowed {
        next.run(request).await
    } else {
        state.log.record(
            Level::Warn,
            module_path!(),
            format_args!("refused a web request naming a host that is not allowed"),
            &[Field::debug("host", &host)],
        );
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
    channels: Vec<ChannelRow>,
    /// Whether hidden channels were asked for (`?hidden=1`).
    show_hidden: bool,
    /// How many of `channels` are hidden.
    hidden_count: usize,
    /// In the order they registered.
    agents: Vec<AgentActivity>,
    /// This server run, if it is recorded.
    current_run: Option<ServerRunInfo>,
    /// How long this server run has been up, as the page says it.
    uptime: Option<String>,
    /// The application's time, for "how long ago".
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
    channel: ChannelInfo,
    /// Hidden: shown to read, with no forms to change it.
    hidden: bool,
    owner: Option<String>,
    /// The operator created the channel, not an agent.
    created_by_operator: bool,
    members: Vec<Member>,
    messages: Vec<Shown>,
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
    agent: AgentInfo,
    channels: Vec<ChannelInfo>,
    message_count: i64,
    memory: Vec<kkt_api_agent::MemoryEntry>,
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

/// A message as the page sends it as JSON (a page of earlier messages, and
/// the live feed), with whether the operator sent it.
#[derive(Serialize)]
struct PageMessage<'a> {
    id: i64,
    channel_id: &'a str,
    channel_name: &'a str,
    agent: &'a str,
    /// The message is from the human operator, not an agent; `agent` is
    /// then `operator`.
    from_operator: bool,
    body: &'a str,
    created_at: &'a str,
    run_id: Option<i64>,
}

impl<'a> PageMessage<'a> {
    fn of(m: &'a Message) -> PageMessage<'a> {
        PageMessage {
            id: m.id,
            channel_id: &m.channel_id,
            channel_name: &m.channel_name,
            agent: &m.agent,
            from_operator: names::is_operator(&m.agent),
            body: &m.body,
            created_at: &m.created_at,
            run_id: m.run_id,
        }
    }
}

/// A message as a page shows it, with whether the operator sent it.
struct Shown {
    m: Message,
    from_operator: bool,
}

impl Shown {
    fn all(messages: Vec<Message>) -> Vec<Shown> {
        messages
            .into_iter()
            .map(|m| Shown {
                from_operator: names::is_operator(&m.agent),
                m,
            })
            .collect()
    }
}

/// A page of a channel's messages, as the channel page's "Show previous"
/// button fetches them.
#[derive(Serialize)]
struct EarlierMessages<'a> {
    /// Oldest first.
    messages: Vec<PageMessage<'a>>,
    /// Set when there are earlier ones still: the `before_id` that reads them.
    older_before_id: Option<i64>,
}

/// The messages just before `before_id`, as JSON, for the channel page to
/// add above the ones it shows.
async fn earlier_messages(
    pages: Pages,
    Path(id): Path<String>,
    Query(params): Query<EarlierParams>,
) -> Response {
    match pages
        .api
        .channel_view(
            &pages.operator,
            id,
            Some(params.before_id),
            pages.settings.page.page_messages,
        )
        .await
    {
        Ok(v) => Json(EarlierMessages {
            messages: v.messages.iter().map(PageMessage::of).collect(),
            older_before_id: v.older_before_id,
        })
        .into_response(),
        Err(e) => match failure(e) {
            Failure::NotFound(message) => (StatusCode::NOT_FOUND, message).into_response(),
            Failure::Invalid(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            Failure::Failed(message) => {
                pages.log.record(
                    Level::Error,
                    module_path!(),
                    format_args!("could not read earlier messages"),
                    &[Field::display("error", &message)],
                );
                (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
            }
        },
    }
}

#[derive(Deserialize)]
struct IndexParams {
    /// `1` lists hidden channels too. Anything else, or nothing, leaves
    /// them out.
    hidden: Option<String>,
}

async fn index(pages: Pages, Query(params): Query<IndexParams>, headers: HeaderMap) -> Response {
    let show_hidden = params.hidden.as_deref() == Some("1");
    let hidden = match show_hidden {
        true => match pages
            .api
            .hidden_channels(&pages.operator, WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
            .await
        {
            Ok(hidden) => ChannelRow::all(hidden, true),
            Err(e) => return pages.error(e, &headers),
        },
        false => Vec::new(),
    };
    match pages
        .api
        .overview(&pages.operator, WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
    {
        Ok(o) => page(
            pages.log.as_ref(),
            IndexPage {
                hidden_count: hidden.len(),
                channels: in_creation_order(ChannelRow::all(o.channels, false), |c| {
                    (&c.activity.info.created_at, &c.activity.info.id)
                })
                .into_iter()
                .chain(in_creation_order(hidden, |c| {
                    (&c.activity.info.created_at, &c.activity.info.id)
                }))
                .collect(),
                show_hidden,
                agents: in_creation_order(o.agents, |a| (&a.info.created_at, &a.info.handle)),
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
            },
        ),
        Err(e) => pages.error(e, &headers),
    }
}

/// A channel on the front page: its activity, and whether it is hidden.
struct ChannelRow {
    activity: ChannelActivity,
    /// Hidden: agents cannot see it, and only the human can read it.
    hidden: bool,
}

impl ChannelRow {
    fn all(channels: Vec<ChannelActivity>, hidden: bool) -> Vec<ChannelRow> {
        channels
            .into_iter()
            .map(|activity| ChannelRow { activity, hidden })
            .collect()
    }

    /// The newest message is from the operator, who has no agent page.
    fn last_from_operator(&self) -> bool {
        self.activity
            .last_speaker
            .as_deref()
            .is_some_and(names::is_operator)
    }

    /// Messages in the window the activity covers.
    fn recent_count(&self) -> i64 {
        self.activity.recent_activity.iter().map(|a| a.count).sum()
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
    runs: Vec<ServerRunInfo>,
    sign_out: bool,
    form_token: String,
}

/// The upgrade history: every server run and what produced it.
async fn history(pages: Pages, headers: HeaderMap) -> Response {
    match pages.api.server_runs(&pages.operator).await {
        Ok(runs) => page(
            pages.log.as_ref(),
            HistoryPage {
                runs,
                sign_out: pages.sign_out,
                form_token: pages.form_token(&headers),
            },
        ),
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

/// A row of a channel page's member list: the member, and an excerpt of its
/// latest message in the channel, if it has sent one.
struct Member {
    agent: AgentInfo,
    latest: Option<Excerpt>,
}

/// The start of a message, on one line, linking to the whole message.
struct Excerpt {
    /// The message's ID.
    id: i64,
    text: String,
    /// The message is longer than `text`.
    cut: bool,
}

impl Member {
    /// Each member with its latest message from `latest`, cut to
    /// `max_chars` characters.
    fn rows(members: Vec<AgentInfo>, latest: &[Message], max_chars: usize) -> Vec<Member> {
        members
            .into_iter()
            .map(|agent| Member {
                latest: latest
                    .iter()
                    .find(|m| m.agent == agent.handle)
                    .map(|m| excerpt(m, max_chars)),
                agent,
            })
            .collect()
    }
}

/// The first `max_chars` characters of a message's body, with its lines and
/// runs of spaces joined into single spaces.
fn excerpt(m: &Message, max_chars: usize) -> Excerpt {
    let flat = m.body.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = flat.chars().count() > max_chars;
    Excerpt {
        id: m.id,
        text: flat.chars().take(max_chars).collect(),
        cut,
    }
}

async fn channel(
    pages: Pages,
    Path(id): Path<String>,
    Query(params): Query<ChannelParams>,
    headers: HeaderMap,
) -> Response {
    match pages
        .api
        .channel_view(
            &pages.operator,
            id,
            params.before_id,
            pages.settings.page.page_messages,
        )
        .await
    {
        Ok(v) => page(
            pages.log.as_ref(),
            ChannelPage {
                hidden: v.hidden,
                owner: v.channel.owner.clone(),
                created_by_operator: names::is_operator(&v.channel.created_by),
                channel: v.channel,
                members: Member::rows(
                    v.members,
                    &v.latest_messages,
                    pages.settings.page.member_excerpt_chars,
                ),
                messages: Shown::all(v.messages),
                older_before_id: v.older_before_id,
                latest: params.before_id.is_none(),
                // A hidden channel gets no new messages.
                live: params.before_id.is_none() && !v.hidden,
                max_messages: pages.settings.page.page_messages,
                sign_out: pages.sign_out,
                form_token: pages.form_token(&headers),
            },
        ),
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
    pages: Pages,
    headers: HeaderMap,
    Change(form): Change<CreateChannelForm>,
) -> Response {
    let open = match form.open.as_str() {
        "true" => true,
        "false" => false,
        _ => {
            return pages.error(RuleError::invalid("choose open or closed"), &headers);
        }
    };
    let members = form
        .members
        .keys()
        .filter_map(|name| name.strip_prefix("member_").map(str::to_string))
        .collect();
    let new = NewChannel {
        name: form.name,
        purpose: Some(form.purpose),
        open,
        members,
    };
    match pages
        .api
        .create_channel_as_operator(pages.actor(), new)
        .await
    {
        Ok(c) => Redirect::to(&format!("/channels/{}", url_path(&c.id))).into_response(),
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
    pages: Pages,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<PurposeForm>,
) -> Response {
    match pages
        .api
        .set_purpose(pages.actor(), id.clone(), form.purpose)
        .await
    {
        Ok(_) => Redirect::to(&format!("/channels/{}", url_path(&id))).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

#[derive(Deserialize)]
struct VisibilityForm {
    open: String,
}

#[derive(Deserialize)]
struct MemberForm {
    handle: String,
}

async fn channel_change(
    pages: Pages,
    id: String,
    headers: HeaderMap,
    change: ChannelChange,
) -> Response {
    let hidden = matches!(change, ChannelChange::Hide);
    match pages
        .api
        .change_channel(pages.actor(), id.clone(), change)
        .await
    {
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
    pages: Pages,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<VisibilityForm>,
) -> Response {
    let open = match form.open.as_str() {
        "true" => true,
        "false" => false,
        _ => {
            return pages.error(RuleError::invalid("open must be true or false"), &headers);
        }
    };
    channel_change(pages, id, headers, ChannelChange::SetOpen(open)).await
}

async fn add_member(
    pages: Pages,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<MemberForm>,
) -> Response {
    channel_change(pages, id, headers, ChannelChange::AddMember(form.handle)).await
}

async fn remove_member(
    pages: Pages,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<MemberForm>,
) -> Response {
    channel_change(pages, id, headers, ChannelChange::RemoveMember(form.handle)).await
}

#[derive(Deserialize)]
struct OwnerForm {
    /// The member to make the owner; empty leaves the channel with none.
    owner: String,
}

/// The human passes a channel's ownership to a member, or clears it.
async fn set_owner(
    pages: Pages,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(form): Change<OwnerForm>,
) -> Response {
    let owner = Some(form.owner.trim().to_string()).filter(|o| !o.is_empty());
    channel_change(pages, id, headers, ChannelChange::SetOwner(owner)).await
}

async fn hide_channel(
    pages: Pages,
    Path(id): Path<String>,
    headers: HeaderMap,
    Change(_): Change<change::NoFields>,
) -> Response {
    channel_change(pages, id, headers, ChannelChange::Hide).await
}

async fn hide_agent(
    pages: Pages,
    Path(handle): Path<String>,
    headers: HeaderMap,
    Change(_): Change<change::NoFields>,
) -> Response {
    match pages.api.hide_agent(pages.actor(), handle).await {
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
    outcome: Option<Outcome>,
    sign_out: bool,
    form_token: String,
}

/// What hiding stale agents did, as the page says it.
struct Outcome {
    /// In the order chosen.
    hidden: Vec<String>,
    /// The agents chosen but kept, in the order chosen.
    kept: Vec<KeptRow>,
}

/// A chosen agent that was kept, and why, as the page says it after the
/// agent's handle.
struct KeptRow {
    handle: String,
    reason: &'static str,
}

impl Outcome {
    fn of(hidden: StaleHidden) -> Outcome {
        Outcome {
            hidden: hidden.hidden,
            kept: hidden
                .kept
                .into_iter()
                .map(|k| KeptRow {
                    handle: k.handle,
                    reason: kept_reason(k.reason),
                })
                .collect(),
        }
    }
}

/// Why a chosen agent was not hidden as stale, as the page says it.
fn kept_reason(reason: KeptReason) -> &'static str {
    match reason {
        KeptReason::Gone => "is already hidden or does not exist",
        KeptReason::Active => "has made a call or started waiting since the list was shown",
        KeptReason::Owner => "owns a channel",
    }
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
fn stale_period(hours: u32) -> Result<std::time::Duration, RuleError> {
    match WEB_STALE_HOURS.contains(&hours) {
        true => Ok(std::time::Duration::from_secs(u64::from(hours) * 60 * 60)),
        false => Err(RuleError::invalid(format!(
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
    pages: Pages,
    Query(params): Query<StaleParams>,
    headers: HeaderMap,
) -> Response {
    let hours = params.hours.unwrap_or(WEB_STALE_DEFAULT_HOURS);
    stale_page(&pages, hours, None, &headers).await
}

#[derive(Deserialize)]
struct HideStaleForm {
    hours: String,
    /// `agent_<handle>` for each agent chosen.
    #[serde(flatten)]
    chosen: std::collections::HashMap<String, String>,
}

/// The human hides the chosen stale agents, then sees the list again with
/// what was hidden and what was kept.
async fn hide_stale_agents(
    pages: Pages,
    headers: HeaderMap,
    Change(form): Change<HideStaleForm>,
) -> Response {
    let Ok(hours) = form.hours.parse::<u32>() else {
        return pages.error(RuleError::invalid("hours must be a whole number"), &headers);
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
    match pages
        .api
        .hide_stale_agents(pages.actor(), idle, chosen)
        .await
    {
        Ok(outcome) => stale_page(&pages, hours, Some(Outcome::of(outcome)), &headers).await,
        Err(e) => pages.error(e, &headers),
    }
}

async fn stale_page(
    pages: &Pages,
    hours: u32,
    outcome: Option<Outcome>,
    headers: &HeaderMap,
) -> Response {
    let idle = match stale_period(hours) {
        Ok(idle) => idle,
        Err(e) => return pages.error(e, headers),
    };
    match pages.api.stale_agents(&pages.operator, idle).await {
        Ok(agents) => page(
            pages.log.as_ref(),
            StalePage {
                periods: Period::all(),
                hours,
                agents,
                outcome,
                sign_out: pages.sign_out,
                form_token: pages.form_token(headers),
            },
        ),
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
    pages: Pages,
    Path(handle): Path<String>,
    headers: HeaderMap,
    Change(NoFields {}): Change<NoFields>,
) -> Response {
    match pages
        .api
        .new_recovery_code(&pages.operator, handle.clone())
        .await
    {
        Ok(code) => (
            [(header::CACHE_CONTROL, "no-store")],
            page(
                pages.log.as_ref(),
                RecoveryCodePage {
                    agent: handle,
                    code,
                    sign_out: pages.sign_out,
                    form_token: pages.form_token(&headers),
                },
            ),
        )
            .into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

async fn agent(pages: Pages, Path(handle): Path<String>, headers: HeaderMap) -> Response {
    match pages.api.agent_view(&pages.operator, handle).await {
        Ok(v) => page(
            pages.log.as_ref(),
            AgentPage {
                agent: v.agent,
                channels: v.channels,
                message_count: v.message_count,
                memory: v.memory,
                sign_out: pages.sign_out,
                form_token: pages.form_token(&headers),
            },
        ),
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
/// afterwards, and `agent_hidden` each channel's. Every event but `message`
/// also carries `toast`, what its toast says (`toast::of`). A `lagged` event
/// means this stream fell behind and missed some, not how many; the page
/// reloads. The stream ends when the application's notices do.
async fn events(pages: Pages, headers: HeaderMap) -> Response {
    let notices = match pages.api.subscribe(&pages.operator).await {
        Ok(notices) => notices,
        Err(e) => return pages.error(e, &headers),
    };
    let log = pages.log.clone();
    let stream = notices_stream(notices)
        .map(move |notice| Ok::<_, Infallible>(sse_event(log.as_ref(), &notice)));
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(pages.settings.keep_alive)
                .text("keep-alive"),
        )
        .into_response()
}

/// `notices` as a stream, which ends when they do.
fn notices_stream(notices: Box<dyn Notices>) -> impl Stream<Item = Notice> {
    futures::stream::unfold(notices, |mut notices| async move {
        let notice = notices.next().await?;
        Some((notice, notices))
    })
}

/// An event's fields, and its toast when it has one.
#[derive(Serialize)]
struct WithToast<'a, T: Serialize> {
    #[serde(flatten)]
    event: &'a T,
    #[serde(skip_serializing_if = "Option::is_none")]
    toast: Option<toast::Toast>,
}

/// `notice` as the stream sends it.
fn sse_event(log: &dyn Log, notice: &Notice) -> Event {
    let toast = toast::of(notice);
    match notice {
        Notice::Message(message) => json_event(log, "message", &PageMessage::of(message)),
        Notice::Agent(agent) => with_toast(log, "agent", agent, toast),
        Notice::Channel(channel) => with_toast(log, "channel", channel, toast),
        Notice::Joined(joined) => with_toast(log, "joined", joined, toast),
        Notice::Left(left) => with_toast(log, "left", left, toast),
        Notice::ChannelHidden(hidden) => with_toast(log, "channel_hidden", hidden, toast),
        Notice::AgentHidden(hidden) => with_toast(log, "agent_hidden", hidden, toast),
        // No number: the lag report carries no count of what was missed.
        Notice::Missed => Event::default().event("lagged").data("missed"),
    }
}

fn with_toast(
    log: &dyn Log,
    name: &str,
    event: &impl Serialize,
    toast: Option<toast::Toast>,
) -> Event {
    json_event(log, name, &WithToast { event, toast })
}

/// An event named `name` carrying `data` as JSON. If that fails, which it
/// should not, the page is told to reload instead.
fn json_event(log: &dyn Log, name: &str, data: &impl serde::Serialize) -> Event {
    match Event::default().event(name).json_data(data) {
        Ok(event) => event,
        Err(e) => {
            log.record(
                Level::Error,
                module_path!(),
                format_args!("could not encode {name} event"),
                &[Field::display("error", &e)],
            );
            Event::default().event("lagged").data("encode error")
        }
    }
}

mod filters {
    /// A stable hue (0 to 359) for an agent's handle, so each agent has its
    /// own color. FNV-1a over the handle's UTF-8 bytes; the page's script
    /// computes the same for live messages.
    #[askama::filter_fn]
    pub fn hue<T: std::fmt::Display>(handle: T, _: &dyn askama::Values) -> askama::Result<u32> {
        Ok(handle_hue(&handle.to_string()))
    }

    pub fn handle_hue(handle: &str) -> u32 {
        let mut hash: u32 = 0x811c_9dc5;
        for byte in handle.bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
        hash % 360
    }

    /// A handle's first character, capitalized, for its avatar.
    #[askama::filter_fn]
    pub fn initial<T: std::fmt::Display>(
        handle: T,
        _: &dyn askama::Values,
    ) -> askama::Result<String> {
        Ok(handle
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

fn page(log: &dyn Log, t: impl Template) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            log.record(
                Level::Error,
                module_path!(),
                format_args!("could not render page"),
                &[Field::display("error", &e)],
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "could not render page").into_response()
        }
    }
}

/// What went wrong, as the page tells it: no such thing, a request that
/// made no sense, or something that broke (whose cause is logged).
enum Failure {
    NotFound(String),
    Invalid(String),
    Failed(String),
}

/// `e`, with the page's advice, as the page tells it.
fn failure(e: RuleError) -> Failure {
    let e = word(e);
    match e.code {
        Code::NotFound => Failure::NotFound(e.message),
        Code::InvalidArgument => Failure::Invalid(e.message),
        _ => Failure::Failed(e.to_string()),
    }
}

/// `e`, with the advice the web page gives for the refusals it can act on:
/// where to pass on or clear an owner. Other errors are as the application
/// gave them.
fn word(e: RuleError) -> RuleError {
    match &e.refusal {
        Some(Refusal::OwnsChannels { handle, channels }) => {
            let message = format!(
                "{handle} owns {}; make another member the owner, or clear the owner, \
                 on each channel's page first",
                channels.join(", ")
            );
            RuleError { message, ..e }
        }
        _ => e,
    }
}

fn error_page(log: &dyn Log, e: RuleError, sign_out: bool, form_token: String) -> Response {
    let (status, message) = match failure(e) {
        Failure::NotFound(message) => (StatusCode::NOT_FOUND, message),
        Failure::Invalid(message) => (StatusCode::BAD_REQUEST, message),
        Failure::Failed(message) => {
            log.record(
                Level::Error,
                module_path!(),
                format_args!("page failed"),
                &[Field::display("error", &message)],
            );
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
            log.record(
                Level::Error,
                module_path!(),
                format_args!("could not render error page"),
                &[Field::display("error", &render_error)],
            );
            (status, "error").into_response()
        }
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod filter_tests {
    use super::filters;

    #[test]
    fn hues_match_the_page_script() {
        // Values printed by base.html's `hue` function under node 22.
        for (handle, hue) in [
            ("amber-falcon", 282),
            ("bold-panther", 299),
            ("swift-jackal", 293),
            ("sunny-heron", 355),
            ("", 61),
            ("é-ünïcode", 304),
        ] {
            assert_eq!(filters::handle_hue(handle), hue, "{handle:?}");
        }
    }

    #[test]
    fn hues_are_stable_and_spread() {
        // The page's script must give the same values (checked with node).
        assert_eq!(
            filters::handle_hue("amber-falcon"),
            filters::handle_hue("amber-falcon")
        );
        assert_eq!(filters::handle_hue(""), 0x811c_9dc5 % 360);
        let hues: std::collections::BTreeSet<u32> = [
            "amber-falcon",
            "bold-panther",
            "swift-jackal",
            "sunny-heron",
        ]
        .iter()
        .map(|n| filters::handle_hue(n))
        .collect();
        assert!(hues.iter().all(|&h| h < 360));
        assert_eq!(hues.len(), 4);
    }
}

const ANNOUNCEMENT_PATH: &str = "/announcement";

#[derive(Template)]
#[template(path = "announcement.html")]
struct ServerAnnouncementPage {
    /// Newest first.
    history: Vec<ServerAnnouncement>,
    current_text: String,
    max_chars: usize,
    sign_out: bool,
    form_token: String,
}
async fn old_announcement_address() -> Response {
    Redirect::permanent(ANNOUNCEMENT_PATH).into_response()
}
async fn server_announcement(pages: Pages, headers: HeaderMap) -> Response {
    match pages.api.server_announcement_history(&pages.operator).await {
        Ok(history) => page(
            pages.log.as_ref(),
            ServerAnnouncementPage {
                current_text: history.first().map_or_else(String::new, |v| v.text.clone()),
                history,
                max_chars: pages.api.limits().max_server_announcement_chars,
                sign_out: pages.sign_out,
                form_token: pages.form_token(&headers),
            },
        ),
        Err(e) => pages.error(e, &headers),
    }
}
#[derive(Deserialize)]
struct AnnouncementForm {
    text: String,
}
async fn save_server_announcement(
    pages: Pages,
    headers: HeaderMap,
    Change(form): Change<AnnouncementForm>,
) -> Response {
    match pages
        .api
        .save_server_announcement(pages.actor(), form.text)
        .await
    {
        Ok(_) => Redirect::to(ANNOUNCEMENT_PATH).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

#[cfg(test)]
mod excerpt_tests {
    use super::{Message, excerpt};

    fn message(body: &str) -> Message {
        Message {
            id: 7,
            channel_id: "ch_test".into(),
            channel_name: "test".into(),
            agent: "amber-falcon".into(),
            body: body.into(),
            created_at: "2026-11-06T00:00:00Z".into(),
            run_id: None,
        }
    }

    #[test]
    fn an_excerpt_is_one_line_cut_to_the_limit_in_characters() {
        let e = excerpt(&message("first  line\n\nsecond\tline"), 80);
        assert_eq!(
            (e.id, e.text.as_str(), e.cut),
            (7, "first line second line", false)
        );
        let e = excerpt(&message("first line\nsecond line"), 10);
        assert_eq!((e.text.as_str(), e.cut), ("first line", true));
        // Characters, not bytes: a cut never splits one.
        let e = excerpt(&message(&"界".repeat(81)), 80);
        assert_eq!((e.text.chars().count(), e.cut), (80, true));
        let e = excerpt(&message(&"界".repeat(80)), 80);
        assert_eq!((e.text.chars().count(), e.cut), (80, false));
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;
    use kkt_log::testing::{CapturedLog, Entry};

    struct Unencodable;
    impl Serialize for Unencodable {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("encoding failed"))
        }
    }

    #[tokio::test]
    async fn encoding_failures_keep_the_reload_event_and_structured_log() {
        let log = CapturedLog::default();
        let event = json_event(&log, "broken", &Unencodable);
        let stream = futures::stream::once(async move { Ok::<_, Infallible>(event) });
        let response = Sse::new(stream).into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("event body");
        assert_eq!(
            String::from_utf8(body.to_vec()).expect("UTF-8"),
            "event: lagged\ndata: encode error\n\n"
        );
        assert_eq!(
            log.entries(),
            vec![Entry {
                level: Level::Error,
                target: "kkt_web",
                message: "could not encode broken event".into(),
                fields: vec![("error", "encoding failed".into())],
            }]
        );
    }
}
