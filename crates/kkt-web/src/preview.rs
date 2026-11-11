//! The preview: a [`Source`] with simulated data, so the page can run and be
//! worked on with no database, no MCP and no agents.
//!
//! The world starts with agents, open and closed channels (one still empty),
//! and a few hundred messages, some very long. [`Preview::run`] then keeps
//! it moving: steady messages, occasional bursts, new agents and new
//! channels. Everything comes from the injected clock and entropy, so a
//! seeded entropy gives the same world and the same feed every time.
//!
//! [`PreviewGate`] is the preview's simulated sign-in, so the page runs with
//! no admission either.

use super::notice::live_event;
use super::settings::MAX_WEB_PAGE_MESSAGES;
use super::settings::SettingError;
use super::source::{
    ActivityCount, Admit, Agent, AgentLane, AgentSummary, AgentView, Channel, ChannelAction,
    ChannelLink, ChannelSummary, ChannelView, Gate, KeptAgent, KeptReason, LaneLimits, LaneRetired,
    LaneSave, LaneSource, LaneSummary, LaneVersion, LaneView, LanesView, LiveEvent, MemoryEntry,
    Message, Overview, ServerRun, SignIn, Source, SourceError, StaleAgent, StaleHidden,
};
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use kkt_clock::{self as clock, Clock, Span, Time};
use kkt_entropy::Entropy;
use kkt_events::{
    AgentHidden, ChannelHidden, ChannelMembers, Events, Joined, Left, NewAgent, NewChannel,
    NewMessage, SwarmEvents,
};
use kkt_names::{self as names, CHANNEL_RANDOM_LEN, MAX_LANE_NAME_CHARS};
use kkt_shutdown::Shutdown;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// The characters the preview's session IDs, form tokens and recovery codes
/// are drawn from: the same 36 the server uses.
const RANDOM_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

// The preview's fixed settings: how big its simulated world is and how it
// moves.

/// Address the preview listens on: next to the server's default port.
pub const PREVIEW_BIND: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7421));
/// Seed for the simulated data when none is given.
pub const PREVIEW_SEED: u64 = 1;
/// Agents in the simulated world at the start.
const PREVIEW_AGENTS: usize = 8;
/// Agents at the start that registered this many hours ago and have made no
/// call since, one per entry, for the stale agents page. The first owns a
/// channel.
const PREVIEW_STALE_HOURS: &[i64] = &[200, 80, 30, 3];
/// Channels at the start. One of them stays empty until the feed reaches it.
const PREVIEW_CHANNELS: usize = 7;
/// Messages at the start, spread over the last few hours. Half go to the
/// first channel, so it has more than one page.
const PREVIEW_MESSAGES: usize = 600;
/// How far back the starting messages go.
const PREVIEW_HISTORY: Duration = Duration::from_secs(6 * 60 * 60);
/// One step in this many is a burst of messages.
const PREVIEW_BURST_ONE_IN: usize = 12;
/// Most messages in a burst.
const PREVIEW_BURST_MAX: usize = 15;
/// One step in this many is a new agent registering.
const PREVIEW_NEW_AGENT_ONE_IN: usize = 25;
/// One step in this many is a new channel.
const PREVIEW_NEW_CHANNEL_ONE_IN: usize = 40;
/// Of the steps that are none of the above, one in this many is an agent
/// leaving a channel.
const PREVIEW_LEAVE_ONE_IN: usize = 15;
/// Of the steps left after that, one in this many is an agent unregistering.
const PREVIEW_UNREGISTER_ONE_IN: usize = 40;
/// Of the steps left after that, one in this many is an owner hiding its
/// channel.
const PREVIEW_HIDE_CHANNEL_ONE_IN: usize = 50;
/// One message in this many is very long.
const PREVIEW_LONG_MESSAGE_ONE_IN: usize = 10;
/// Characters in a simulated recovery code after its `arc_` prefix.
const PREVIEW_RECOVERY_CODE_CHARS: usize = 26;
/// How long a sign-in to the preview lasts, as the server's default
/// `web_session_lifetime`.
pub const PREVIEW_SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
/// Characters in a simulated session ID or form token, as in the server's
/// (`kkt_admission::PREVIEW_SESSION_ID_CHARS`, which kkt-web may not use).
const PREVIEW_SESSION_ID_CHARS: usize = 32;
/// Shortest key the preview's gate takes, in bytes, as the server's
/// (`kkt_admission::check_access_key`).
const PREVIEW_MIN_KEY_BYTES: usize = 32;

/// What the preview's operator can change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewSettings {
    /// Steps of the simulated feed per minute. Most steps are one message.
    pub steps_per_minute: u32,
    /// The most messages one page shows, as the server's `web_page_messages`.
    pub page_messages: usize,
}

impl PreviewSettings {
    pub const DEFAULT: PreviewSettings = PreviewSettings {
        steps_per_minute: 30,
        // As kkt-web's default page size.
        page_messages: 50,
    };

    /// Checks the settings. The preview refuses to start with ones that fail.
    pub fn check(&self) -> Result<(), SettingError> {
        if self.steps_per_minute == 0 || self.steps_per_minute > 60_000 {
            return Err(SettingError::new(
                "rate",
                "must be from 1 to 60000 steps per minute",
            ));
        }
        if self.page_messages == 0 || self.page_messages > MAX_WEB_PAGE_MESSAGES {
            return Err(SettingError::new(
                "page_messages",
                format!("must be from 1 to {MAX_WEB_PAGE_MESSAGES}"),
            ));
        }
        Ok(())
    }
}

/// Checks a key for the preview's gate as the server checks its keys
/// (`kkt_admission::check_access_key`, which kkt-web may not use), with the
/// same rules and words: at least [`PREVIEW_MIN_KEY_BYTES`] long, and only
/// printable ASCII with no spaces.
fn check_preview_key(key: &str) -> Result<(), SettingError> {
    if key.is_empty() {
        return Err(SettingError::new("access_key_file", "must not be empty"));
    }
    if !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(SettingError::new(
            "access_key_file",
            "must hold only printable ASCII with no spaces",
        ));
    }
    if key.len() < PREVIEW_MIN_KEY_BYTES {
        return Err(SettingError::new(
            "access_key_file",
            format!("must hold a key of at least {PREVIEW_MIN_KEY_BYTES} characters"),
        ));
    }
    Ok(())
}

/// The simulated server run that is running, which every row belongs to.
/// The runs before it make the upgrade history.
const RUN_ID: i64 = 4;

/// The upgrade history, newest first: this run, started at `start`, and
/// three before it, with values as long as real ones (full commits, a
/// `-dirty` mark, many settings, a long allowed origin) so the history page
/// can be checked at a narrow width.
fn runs(start: Time, settings: &PreviewSettings) -> Vec<ServerRun> {
    let settings_text = |extra: &str| {
        format!(
            "max_response_bytes=65536\ndefault_limit=20\nmax_limit=100\nmax_body_bytes=8192\n\
             sends_per_minute=60\nregistrations_per_minute=10\nmax_agents=500\n\
             default_wait_ms=30000\nmax_wait_ms=50000\nweb_page_messages=200\nlog_level=info\n\
             allowed_origin=https://a-rather-long-machine-name.tail1234abcd.ts.net\n\
             allowed_host=localhost\n{extra}"
        )
    };
    [
        (
            RUN_ID,
            Span::ZERO,
            "preview",
            "preview".to_string(),
            settings_text(&format!(
                "preview steps_per_minute={}",
                settings.steps_per_minute
            )),
        ),
        (
            3,
            Span::days(2),
            "0.5.0",
            "4f1c2a9e7b3d5f60a8c1e2d3b4a5968778695a4b".to_string(),
            settings_text("upgrade_backups_kept=5"),
        ),
        (
            2,
            Span::days(9),
            "0.4.1",
            "9a8b7c6d5e4f30211203a4b5c6d7e8f9a0b1c2d3-dirty".to_string(),
            settings_text(""),
        ),
        (
            1,
            Span::days(23),
            "0.4.0",
            "unknown".to_string(),
            settings_text(""),
        ),
    ]
    .into_iter()
    .map(|(id, before, version, git_commit, settings)| ServerRun {
        id,
        started_at: clock::to_text(start - before),
        version: version.into(),
        git_commit,
        settings,
        instructions_sha256: format!("{id:064x}"),
    })
    .collect()
}

/// Channel names, purposes and kinds the world draws from, in order.
const CHANNELS: &[(&str, &str, bool)] = &[
    ("general", "Everyone, everything", true),
    (
        "design-review",
        "Sketches and feedback on the web page",
        true,
    ),
    (
        "ops-alerts",
        "Automated alerts from the deploy agents",
        true,
    ),
    ("secret-lab", "Experiments not ready to share", false),
    ("release-planning", "What ships next and when", true),
    ("pairing", "Two agents working one problem", false),
    ("quiet-corner", "Nothing has happened here yet", true),
    ("incident-1729", "Tracking one outage end to end", true),
    ("research", "Papers, links and summaries", true),
    ("backchannel", "Coordinators only", false),
];

/// The name and purpose of the channel the world starts with already hidden.
const HIDDEN_CHANNEL: (&str, &str) = ("old-experiments", "Work wrapped up and put away");

/// The memory entries simulated agents keep, in name order.
const PREVIEW_MEMORY: &[(&str, &str)] = &[
    (
        "handoff",
        "Working on the TOML parser rewrite in ~/src/parser.\n\
         Done: tokenizer, tests green.\n\
         Next: error messages; bold-magpie reviews.",
    ),
    ("rules", "Pushes need approval from the owner."),
];

/// Descriptions simulated agents may give of themselves.
const DESCRIPTIONS: &[&str] = &[
    "Claude Code, working in ~/src/kashkeshet. git push needs approval from my owner.",
    "Codex. Reviews only; no test runs here.",
    "OpenCode helper for the docs.",
];

const LINES: &[&str] = &[
    "Starting on the task now.",
    "Done. PR is open for review.",
    "Can someone check the failing test on main?",
    "I'll take that one.",
    "Rebased onto next; all checks clean.",
    "Blocked: I need access to the staging database.",
    "That looks right to me. Approved.",
    "Changes requested: the error is swallowed in the retry loop.",
    "Here's the summary you asked for.",
    "Ack.",
    "Moving this to #release-planning.",
    "The build is green again.",
    "Does anyone know why the cache misses doubled overnight?",
    "Pushed a fix; please re-review.",
    "Thanks, merging.",
    "Heads-up: I'm restarting the worker pool in five minutes.",
    "Found it: the timeout was in seconds, not milliseconds.",
    "Can we split this into two PRs?",
    "Running the benchmarks now; results in ten minutes.",
    "Here are the numbers: p50 12 ms, p99 48 ms.",
];

const PARAGRAPHS: &[&str] = &[
    "I went through the logs from the last three runs and the pattern is consistent: the first request after an idle period takes far longer than the rest, which points at connection setup rather than the query itself.",
    "Two options. We can keep a warm pool, which costs memory but fixes the latency outright, or we can accept the first slow call and document it. I lean toward the pool, sized small.",
    "Steps to reproduce:\n1. Start the server with an empty database.\n2. Register two agents.\n3. Have both send at the same moment.\n4. Watch the second one wait for the lock.",
    "fn main() {\n    let server = Server::new(settings);\n    server.run().await?;\n}",
    "For the record, the decision was to keep the old behavior for bare invocations and only change the default under the new subcommand, so nothing that already works breaks.",
    "I've also left notes in the PR description about what I could not test locally, mostly around the macOS service manager and the Safari cookie behavior.",
];

/// A simulated [`Gate`] for the preview and the page tests, so they need no
/// admission. It holds one key, fixed when it is made, or none for an open
/// page. Sessions live in memory for a fixed lifetime, timed by the injected
/// clock; session IDs and form tokens come from the injected entropy.
///
/// It behaves like the server's gate for an open page, a wrong key, session
/// lifetime, form tokens and signing out, with the same refusal texts. It is
/// not a security boundary: it compares keys and tokens plainly and keeps
/// session IDs as they are.
pub struct PreviewGate {
    key: Option<String>,
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
    lifetime: Duration,
    /// The form token when there is no key, so no sessions to tie one to.
    /// Drawn when the gate is made.
    open_form_token: String,
    /// Live sessions by ID.
    sessions: Mutex<HashMap<String, PreviewSession>>,
}

struct PreviewSession {
    started: Time,
    form_token: String,
}

impl PreviewGate {
    /// A gate with no key: every page is open, with no login form.
    pub fn open(
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
        lifetime: Duration,
    ) -> PreviewGate {
        PreviewGate {
            key: None,
            open_form_token: entropy.random_chars(RANDOM_CHARS, PREVIEW_SESSION_ID_CHARS),
            clock,
            entropy,
            lifetime,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// A gate that lets in viewers who sign in with the key in `text`,
    /// without surrounding whitespace, for sessions lasting `lifetime`.
    /// Fails, as the server does, if the key is not one the server's check
    /// accepts (`check_preview_key`).
    pub fn with_key(
        text: &str,
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
        lifetime: Duration,
    ) -> Result<PreviewGate, SettingError> {
        let key = text.trim();
        check_preview_key(key)?;
        Ok(PreviewGate {
            key: Some(key.to_string()),
            ..PreviewGate::open(clock, entropy, lifetime)
        })
    }

    /// The form token of the live session with ID `id`, if there is one. An
    /// expired session is dropped.
    fn live_form_token(&self, id: &str) -> Option<String> {
        let now = self.clock.now();
        let mut sessions = self.sessions();
        match sessions.get(id) {
            Some(s) if self.within_lifetime(s, now) => Some(s.form_token.clone()),
            Some(_) => {
                sessions.remove(id);
                None
            }
            None => None,
        }
    }

    fn within_lifetime(&self, session: &PreviewSession, now: Time) -> bool {
        // A clock that moved backwards gives a negative age, counted as zero.
        let age = (now - session.started).to_std().unwrap_or(Duration::ZERO);
        age < self.lifetime
    }

    /// The session table. Each change to it is a single insert or removal,
    /// so it stays usable after a panic elsewhere.
    fn sessions(&self) -> MutexGuard<'_, HashMap<String, PreviewSession>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The preview has one view, so its viewers are all the same: `()`.
impl Gate for PreviewGate {
    type Viewer = ();

    fn sign_in_required(&self) -> bool {
        self.key.is_some()
    }

    fn admits<'a>(&'a self, session_id: Option<&'a str>) -> BoxFuture<'a, Admit<()>> {
        let admit = if !self.sign_in_required() {
            Admit::Yes(())
        } else {
            match session_id.map(|id| self.live_form_token(id)) {
                None => Admit::No("sign in first".into()),
                Some(Some(_)) => Admit::Yes(()),
                Some(None) => Admit::No("the session has ended; sign in again".into()),
            }
        };
        Box::pin(std::future::ready(admit))
    }

    fn sign_in(&self, key: &str) -> SignIn {
        if self.key.as_deref().is_some_and(|k| k != key.trim()) {
            return SignIn::Refused("the access key is wrong".into());
        }
        let session_id = self
            .entropy
            .random_chars(RANDOM_CHARS, PREVIEW_SESSION_ID_CHARS);
        let session = PreviewSession {
            started: self.clock.now(),
            form_token: self
                .entropy
                .random_chars(RANDOM_CHARS, PREVIEW_SESSION_ID_CHARS),
        };
        let now = session.started;
        let mut sessions = self.sessions();
        sessions.retain(|_, s| self.within_lifetime(s, now));
        sessions.insert(session_id.clone(), session);
        SignIn::Started {
            session_id,
            lifetime: self.lifetime,
        }
    }

    fn sign_out(&self, session_id: &str) {
        self.sessions().remove(session_id);
    }

    fn form_token(&self, session_id: Option<&str>) -> Option<String> {
        if !self.sign_in_required() {
            return Some(self.open_form_token.clone());
        }
        self.live_form_token(session_id?)
    }

    fn form_token_matches(&self, session_id: Option<&str>, presented: &str) -> bool {
        self.form_token(session_id).as_deref() == Some(presented)
    }
}

/// Simulated data for the web page.
pub struct Preview {
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
    settings: PreviewSettings,
    events: SwarmEvents,
    world: Mutex<World>,
}

/// What one step of the feed did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Happening {
    Message,
    Burst(usize),
    NewAgent(String),
    NewChannel(String),
    /// This agent left a channel.
    Left(String),
    /// This agent unregistered.
    Unregistered(String),
    /// The channel with this ID was hidden by its owner.
    ChannelHidden(String),
}

/// What a step or a change publishes, in order, once the world is unlocked.
enum Out {
    Message(Message),
    Agent(NewAgent),
    Channel(NewChannel),
    Joined(Joined),
    Left(Left),
    ChannelHidden(ChannelHidden),
    AgentHidden(AgentHidden),
}

struct World {
    retired_names: std::collections::HashSet<String>,
    agents: Vec<Agent>,
    channels: Vec<SimChannel>,
    /// Channels the operator hid, in the order they were hidden. As on the
    /// server, they have no members, and their messages are kept.
    hidden: Vec<SimChannel>,
    /// Every message, by ID.
    messages: Vec<Message>,
    runs: Vec<ServerRun>,
    server_announcement: Vec<super::source::ServerAnnouncementVersion>,
    /// In name order, each with its history newest first.
    lanes: Vec<LaneView>,
    next_id: i64,
    /// How many of `CHANNELS` are used so far.
    channels_used: usize,
}

struct SimChannel {
    channel: Channel,
    /// In the order they joined.
    members: Vec<String>,
    /// The agent that created it, until the operator passes the ownership
    /// on or clears it; none if the operator created it.
    owner: Option<String>,
}

impl Preview {
    /// A new world from `entropy`, with times leading up to the clock's now.
    /// Live feeds end when `shutdown` stops. Refused, naming the constant,
    /// if `PREVIEW_HISTORY` is too long for a time span.
    pub fn new(
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
        settings: PreviewSettings,
        shutdown: &Shutdown,
    ) -> Result<Preview, SettingError> {
        let span = |name: &'static str, d: Duration| {
            Span::from_std(d).map_err(|e| SettingError::new(name, e.to_string()))
        };
        let history = span("PREVIEW_HISTORY", PREVIEW_HISTORY)?;
        let world = World::start(clock.now(), history, entropy.as_ref(), &settings);
        Ok(Preview {
            clock,
            entropy,
            settings,
            // The preview is one made-up swarm.
            events: Events::new(kkt_events::Settings::DEFAULT)
                .until_stopped(shutdown.clone())
                .in_swarm("sw_preview"),
            world: Mutex::new(world),
        })
    }

    /// Keeps the world moving, one step every `60 / steps_per_minute`
    /// seconds by the clock, until `shutdown` stops.
    pub async fn run(&self, shutdown: Shutdown) {
        let interval =
            Span::milliseconds(60_000 / i64::from(self.settings.steps_per_minute.max(1)));
        let mut next = self.clock.now() + interval;
        loop {
            tokio::select! {
                biased;
                () = shutdown.stopped() => return,
                () = self.clock.sleep_until(next) => {}
            }
            self.step();
            // After a long pause (a laptop asleep), carry on from now rather
            // than catching up all at once.
            next = (next + interval).max(self.clock.now());
        }
    }

    /// One step of the feed: usually one message, sometimes a burst, a new
    /// agent or a new channel. Each message is published to live feeds.
    pub fn step(&self) -> Happening {
        let now = self.clock.now();
        let e = self.entropy.as_ref();
        let mut world = self.world();
        // Administration may leave no agents or channels. The simulated feed
        // can create fresh ones without reviving a hidden avatar/channel.
        if world.agents.is_empty() {
            world.add_agent(now, e);
        }
        if world.channels.is_empty() {
            world.add_channel(now, e);
        }
        let mut out = Vec::new();
        let happening = if e.below(PREVIEW_NEW_CHANNEL_ONE_IN) == 0 {
            let id = world.add_channel(now, e);
            if let Some(c) = world.channels.iter().find(|c| c.channel.id == id) {
                out.push(Out::Channel(NewChannel {
                    id: id.clone(),
                    name: c.channel.name.clone(),
                    open: c.channel.open,
                    created_by: c.channel.created_by.clone(),
                    created_at: c.channel.created_at.clone(),
                    member_count: c.members.len() as i64,
                }));
            }
            out.push(Out::Message(world.post(now, e, Some(&id))));
            Happening::NewChannel(id)
        } else if e.below(PREVIEW_NEW_AGENT_ONE_IN) == 0 {
            let name = world.add_agent(now, e);
            out.push(Out::Agent(NewAgent {
                handle: name.clone(),
                created_at: clock::to_text(now),
            }));
            // Every channel a new agent is in, it has just joined.
            for c in world.channels.iter().filter(|c| c.members.contains(&name)) {
                out.push(Out::Joined(Joined {
                    agent: name.clone(),
                    channel_id: c.channel.id.clone(),
                    channel_name: c.channel.name.clone(),
                    member_count: c.members.len() as i64,
                }));
            }
            out.extend(
                world
                    .post_as(now, &name, "Hello, I just registered.")
                    .map(Out::Message),
            );
            Happening::NewAgent(name)
        } else if e.below(PREVIEW_BURST_ONE_IN) == 0 {
            let count = 3 + e.below(PREVIEW_BURST_MAX - 2);
            let channel = world.busy_channel(e);
            for _ in 0..count {
                out.push(Out::Message(world.post(now, e, Some(&channel))));
            }
            Happening::Burst(count)
        } else if e.below(PREVIEW_LEAVE_ONE_IN) == 0
            && let Some(left) = world.leave(e)
        {
            let agent = left.agent.clone();
            out.push(Out::Left(left));
            Happening::Left(agent)
        } else if e.below(PREVIEW_UNREGISTER_ONE_IN) == 0 && world.agents.len() > 1 {
            let name = world.agents[e.below(world.agents.len())].handle.clone();
            let body = format!("{name} unregistered and left the channel. Its messages are kept.");
            out.extend(world.remove_avatar(now, &name, &name, &body, "unregistered"));
            Happening::Unregistered(name)
        } else if e.below(PREVIEW_HIDE_CHANNEL_ONE_IN) == 0
            && let Some((index, owner)) = world.owned_channel(e)
        {
            let id = world.channels[index].channel.id.clone();
            out.extend(world.hide(now, index, &owner, &owner));
            Happening::ChannelHidden(id)
        } else {
            out.push(Out::Message(world.post(now, e, None)));
            Happening::Message
        };
        drop(world);
        self.publish(out);
        happening
    }

    fn publish(&self, out: Vec<Out>) {
        for item in out {
            match item {
                Out::Message(m) => self.publish_message(m),
                Out::Agent(a) => self.events.publish_agent(a),
                Out::Channel(c) => self.events.publish_channel(c),
                Out::Joined(j) => self.events.publish_joined(j),
                Out::Left(l) => self.events.publish_left(l),
                Out::ChannelHidden(h) => self.events.publish_channel_hidden(h),
                Out::AgentHidden(h) => self.events.publish_agent_hidden(h),
            }
        }
    }

    fn publish_message(&self, m: Message) {
        self.events.publish(NewMessage {
            id: m.id,
            channel_id: m.channel_id,
            channel_name: m.channel_name,
            agent: m.agent,
            body: m.body,
            created_at: m.created_at,
            run_id: m.run_id,
        });
    }

    /// The start of the last `window`, as a stored time. Fails if `window`
    /// is too long for a time span.
    fn activity_since(&self, window: Duration) -> Result<String, SourceError> {
        let window = Span::from_std(window).map_err(|e| SourceError::Invalid(e.to_string()))?;
        Ok(clock::to_text(self.clock.now() - window))
    }

    fn world(&self) -> MutexGuard<'_, World> {
        // A poisoned lock means a step panicked part way; the world is still
        // readable, and it is only simulated data.
        self.world.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Source for Preview {
    fn overview(
        &self,
        window: Duration,
        bucket: Duration,
    ) -> BoxFuture<'_, Result<Overview, SourceError>> {
        let now = self.clock.now();
        let overview = self
            .activity_since(window)
            .map(|since| self.world().overview(&since, bucket, clock::to_text(now)));
        Box::pin(async move { overview })
    }

    fn hidden_channels(
        &self,
        window: Duration,
        bucket: Duration,
    ) -> BoxFuture<'_, Result<Vec<ChannelSummary>, SourceError>> {
        let hidden = self
            .activity_since(window)
            .map(|since| self.world().hidden_channels(&since, bucket));
        Box::pin(async move { hidden })
    }

    fn server_runs(&self) -> BoxFuture<'_, Result<Vec<ServerRun>, SourceError>> {
        let runs = self.world().runs.clone();
        Box::pin(async move { Ok(runs) })
    }

    fn channel(
        &self,
        id: String,
        before_id: Option<i64>,
        page_messages: usize,
    ) -> BoxFuture<'_, Result<ChannelView, SourceError>> {
        let world = self.world();
        let view = world.channel_view(&id, before_id, page_messages);
        Box::pin(async move { view })
    }

    fn agent(&self, handle: String) -> BoxFuture<'_, Result<AgentView, SourceError>> {
        let world = self.world();
        let view = world.agent_view(&handle);
        Box::pin(async move { view })
    }

    fn create_channel(
        &self,
        name: String,
        purpose: String,
        open: bool,
        members: Vec<String>,
    ) -> BoxFuture<'_, Result<String, SourceError>> {
        let result = self.world().create_channel(
            self.clock.now(),
            self.entropy.as_ref(),
            name,
            purpose,
            open,
            members,
        );
        let result = result.map(|(channel, notice)| {
            self.events.publish_channel(NewChannel {
                id: channel.id.clone(),
                name: channel.name,
                open: channel.open,
                created_by: channel.created_by,
                created_at: channel.created_at,
                member_count: channel.member_count,
            });
            self.publish_message(notice);
            channel.id
        });
        Box::pin(async move { result })
    }

    fn set_purpose(
        &self,
        channel_id: String,
        purpose: String,
    ) -> BoxFuture<'_, Result<(), SourceError>> {
        let now = self.clock.now();
        let said = self.world().set_purpose(now, &channel_id, &purpose);
        let said = said.map(|m| self.publish_message(m));
        Box::pin(async move { said })
    }

    fn change_channel(
        &self,
        channel_id: String,
        change: ChannelAction,
    ) -> BoxFuture<'_, Result<(), SourceError>> {
        let result = self
            .world()
            .change_channel(self.clock.now(), &channel_id, change);
        let result = result.map(|out| self.publish(out));
        Box::pin(async move { result })
    }

    fn hide_agent(&self, handle: String) -> BoxFuture<'_, Result<(), SourceError>> {
        let result = self.world().hide_agent(self.clock.now(), &handle);
        let result = result.map(|out| self.publish(out));
        Box::pin(async move { result })
    }

    fn stale_agents(&self, idle: Duration) -> BoxFuture<'_, Result<Vec<StaleAgent>, SourceError>> {
        let result = stale_cutoff(self.clock.now(), idle).map(|cutoff| {
            let world = self.world();
            let mut stale: Vec<StaleAgent> = world
                .agents
                .iter()
                .filter(|a| is_stale(a, &cutoff))
                .map(|a| world.stale_agent(a))
                .collect();
            stale.sort_by(|a, b| last_call(&a.agent).cmp(last_call(&b.agent)));
            stale
        });
        Box::pin(async move { result })
    }

    fn hide_stale_agents(
        &self,
        idle: Duration,
        chosen: Vec<String>,
    ) -> BoxFuture<'_, Result<StaleHidden, SourceError>> {
        let now = self.clock.now();
        let result = stale_cutoff(now, idle).and_then(|cutoff| {
            let mut world = self.world();
            let mut outcome = StaleHidden {
                hidden: Vec::new(),
                kept: Vec::new(),
            };
            let mut out = Vec::new();
            for handle in chosen {
                if outcome.hidden.contains(&handle)
                    || outcome.kept.iter().any(|k| k.handle == handle)
                {
                    continue;
                }
                let reason = match world.agents.iter().find(|a| a.handle == handle) {
                    None => Some(KeptReason::Gone),
                    Some(a) if !is_stale(a, &cutoff) => Some(KeptReason::Active),
                    Some(_) if world.owns_any(&handle) => Some(KeptReason::Owner),
                    Some(_) => None,
                };
                match reason {
                    Some(reason) => outcome.kept.push(KeptAgent { handle, reason }),
                    None => {
                        out.extend(world.hide_agent(now, &handle)?);
                        outcome.hidden.push(handle);
                    }
                }
            }
            Ok((outcome, out))
        });
        let result = result.map(|(outcome, out)| {
            self.publish(out);
            outcome
        });
        Box::pin(async move { result })
    }

    fn new_recovery_code(&self, handle: String) -> BoxFuture<'_, Result<String, SourceError>> {
        let known = self.world().agents.iter().any(|a| a.handle == handle);
        let code = match known {
            true => Ok(format!(
                "arc_{}",
                self.entropy
                    .random_chars(RANDOM_CHARS, PREVIEW_RECOVERY_CODE_CHARS)
            )),
            false => Err(SourceError::NotFound(format!("no agent {handle}"))),
        };
        Box::pin(async move { code })
    }

    fn server_announcement(
        &self,
    ) -> BoxFuture<'_, Result<super::source::ServerAnnouncementView, SourceError>> {
        let view = super::source::ServerAnnouncementView {
            history: self.world().server_announcement.clone(),
            max_chars: LIMITS.max_server_announcement_chars,
        };
        Box::pin(async move { Ok(view) })
    }
    fn save_server_announcement(&self, text: String) -> BoxFuture<'_, Result<(), SourceError>> {
        let max = LIMITS.max_server_announcement_chars;
        if text.chars().count() > max {
            return Box::pin(async move {
                Err(SourceError::Invalid(format!(
                    "the server announcement must be at most {max} characters"
                )))
            });
        }
        let mut world = self.world();
        let version = world
            .server_announcement
            .first()
            .map_or(1, |v| v.version + 1);
        world.server_announcement.insert(
            0,
            super::source::ServerAnnouncementVersion {
                version,
                text,
                saved_at: clock::to_text(self.clock.now()),
                run_id: RUN_ID,
                saved_by_admin: None,
            },
        );
        Box::pin(async { Ok(()) })
    }

    fn lanes(&self) -> BoxFuture<'_, Result<LanesView, SourceError>> {
        let view = LanesView {
            lanes: self
                .world()
                .lanes
                .iter()
                .map(|l| LaneSummary {
                    name: l.name.clone(),
                    created_at: l.created_at.clone(),
                    retired: l.retired.clone(),
                    current: l.history[0].clone(),
                })
                .collect(),
            limits: lane_limits(),
        };
        Box::pin(async move { Ok(view) })
    }

    fn lane(&self, name: String) -> BoxFuture<'_, Result<LaneView, SourceError>> {
        let found = self
            .world()
            .lanes
            .iter()
            .find(|l| l.name == name)
            .cloned()
            .ok_or_else(|| SourceError::NotFound(format!("no lane named {name}")));
        Box::pin(async move { found })
    }

    fn save_lane(&self, save: LaneSave, create: bool) -> BoxFuture<'_, Result<(), SourceError>> {
        let now = clock::to_text(self.clock.now());
        let result = self.world().save_lane(save, create, now);
        Box::pin(async move { result })
    }

    fn retire_lane(&self, name: String) -> BoxFuture<'_, Result<(), SourceError>> {
        let now = clock::to_text(self.clock.now());
        let mut world = self.world();
        let result = match world.lanes.iter_mut().find(|l| l.name == name) {
            Some(lane) => {
                if lane.retired.is_none() {
                    lane.retired = Some(LaneRetired {
                        at: now,
                        by_admin: None,
                    });
                }
                Ok(())
            }
            None => Err(SourceError::NotFound(format!("no lane named {name}"))),
        };
        Box::pin(async move { result })
    }

    fn live(&self) -> BoxStream<'static, LiveEvent> {
        self.events
            .subscribe()
            .into_stream()
            .map(live_event)
            .boxed()
    }
}

/// The limits the preview keeps to, as a server keeps to its defaults.
struct PreviewLimits {
    max_channel_name_chars: usize,
    max_purpose_chars: usize,
    max_server_announcement_chars: usize,
    max_lane_description_chars: usize,
    max_lane_chars: usize,
}

/// The preview's limits: a server's defaults.
const LIMITS: PreviewLimits = PreviewLimits {
    max_channel_name_chars: 64,
    max_purpose_chars: 500,
    max_server_announcement_chars: 4000,
    max_lane_description_chars: 200,
    max_lane_chars: 8000,
};

/// The server's lane limits, at their defaults.
fn lane_limits() -> LaneLimits {
    LaneLimits {
        max_name_chars: MAX_LANE_NAME_CHARS,
        max_description_chars: LIMITS.max_lane_description_chars,
        max_text_chars: LIMITS.max_lane_chars,
    }
}

impl World {
    /// Saves a lane as the server would, with the main checks: the name, a
    /// one-line description, instructions, and the limits.
    fn save_lane(&mut self, save: LaneSave, create: bool, now: String) -> Result<(), SourceError> {
        let limits = lane_limits();
        names::check_lane_name(&save.name, MAX_LANE_NAME_CHARS).map_err(SourceError::Invalid)?;
        let description = save.description.trim().to_string();
        if description.is_empty() || description.contains(['\n', '\r']) {
            return Err(SourceError::Invalid(
                "the lane needs a one-line description".into(),
            ));
        }
        if description.chars().count() > limits.max_description_chars
            || save.text.chars().count() > limits.max_text_chars
        {
            return Err(SourceError::Invalid("the lane is too long".into()));
        }
        if save.text.trim().is_empty() {
            return Err(SourceError::Invalid("the lane needs instructions".into()));
        }
        let version = |number| LaneVersion {
            version: number,
            description: description.clone(),
            text: save.text.clone(),
            saved_at: now.clone(),
            run_id: RUN_ID,
            saved_by_admin: None,
            source: None,
        };
        match self.lanes.iter_mut().find(|l| l.name == save.name) {
            Some(_) if create => Err(SourceError::Invalid(format!(
                "a lane named {} already exists",
                save.name
            ))),
            Some(lane) if lane.retired.is_some() => Err(SourceError::Invalid(format!(
                "the lane {} is retired, so it cannot be changed",
                save.name
            ))),
            Some(lane) => {
                let current = &lane.history[0];
                if current.description != description || current.text != save.text {
                    let next = version(current.version + 1);
                    lane.history.insert(0, next);
                }
                Ok(())
            }
            None if !create => Err(SourceError::NotFound(format!(
                "no lane named {}",
                save.name
            ))),
            None => {
                self.lanes.push(LaneView {
                    name: save.name.clone(),
                    created_at: now.clone(),
                    retired: None,
                    history: vec![version(1)],
                    limits,
                });
                self.lanes.sort_by(|a, b| a.name.cmp(&b.name));
                Ok(())
            }
        }
    }

    fn start(now: Time, history: Span, e: &dyn Entropy, settings: &PreviewSettings) -> World {
        let start = now - history;
        let mut world = World {
            retired_names: std::collections::HashSet::new(),
            server_announcement: Vec::new(),
            lanes: vec![LaneView {
                name: "orchestrator".into(),
                created_at: clock::to_text(start),
                retired: None,
                history: vec![
                    LaneVersion {
                        version: 2,
                        description: "Coordinates workers on this server.".into(),
                        text: "Post each task in #work, give it to one worker, and check its \
                               pull request before starting the next. Thank the worker."
                            .into(),
                        saved_at: clock::to_text(start),
                        run_id: RUN_ID,
                        saved_by_admin: None,
                        // Imported with `kkt lanes import`.
                        source: Some(LaneSource {
                            folder: "/home/sam/lanes".into(),
                            commit: Some("3f2a9c1e0b7d4a6f8e5c2b1a0d9e8f7c6b5a4d3e".into()),
                        }),
                    },
                    LaneVersion {
                        version: 1,
                        description: "Coordinates workers on this server.".into(),
                        text: "Post each task in #work, give it to one worker, and check its \
                               pull request before starting the next."
                            .into(),
                        saved_at: clock::to_text(start),
                        run_id: RUN_ID,
                        saved_by_admin: None,
                        source: None,
                    },
                ],
                limits: lane_limits(),
            }],
            agents: Vec::new(),
            channels: Vec::new(),
            hidden: Vec::new(),
            messages: Vec::new(),
            runs: runs(start, settings),
            next_id: 1,
            channels_used: 0,
        };
        for _ in 0..PREVIEW_AGENTS {
            world.add_agent(start, e);
        }
        // The first agent holds the newest version of the lane, the second
        // an older one, so the pages show both.
        for (agent, version) in world.agents.iter_mut().zip([2, 1]) {
            agent.lane = Some(AgentLane {
                name: "orchestrator".into(),
                version,
                current_version: 2,
            });
        }
        for _ in 0..PREVIEW_CHANNELS {
            world.add_channel(start, e);
        }
        // Spread the starting messages over the history: half in the first
        // channel, so it has older pages, and none in the last
        // (quiet-corner), so it starts empty.
        let busy = world.channels.len().saturating_sub(1).max(1);
        let step = history / i32::try_from(PREVIEW_MESSAGES).unwrap_or(i32::MAX).max(1);
        for k in 0..PREVIEW_MESSAGES {
            let at = start + step * i32::try_from(k).unwrap_or(i32::MAX);
            let index = match e.below(2) {
                0 => 0,
                _ => e.below(busy),
            };
            let channel = world.channels[index].channel.id.clone();
            world.post(at, e, Some(&channel));
        }
        world.add_hidden_channel(now, e);
        for agent in &mut world.agents {
            let minutes = i64::try_from(e.below(90)).unwrap_or(0);
            agent.last_seen = Some(clock::to_text(now - Span::minutes(minutes)));
        }
        // Agents that registered and went quiet, the first owning the empty
        // channel, so the stale agents page has owners and non-owners.
        for (k, &hours) in PREVIEW_STALE_HOURS.iter().enumerate() {
            let name = world.add_agent(now - Span::hours(hours), e);
            let last = world.channels.len().checked_sub(1);
            if let (0, Some(last)) = (k, last) {
                world.join(last, &name);
                world.channels[last].owner = Some(name);
            }
        }
        world
    }

    /// A channel that was used for a while and then hidden by the operator,
    /// so the page has one to show when asked.
    fn add_hidden_channel(&mut self, now: Time, e: &dyn Entropy) {
        let id = self.add_channel(now - Span::hours(2), e);
        let index = self.channels.len() - 1;
        let (name, purpose) = HIDDEN_CHANNEL;
        self.channels[index].channel.name = name.into();
        self.channels[index].channel.purpose = Some(purpose.into());
        for minutes in [90, 80, 70, 60, 50, 40] {
            self.post(now - Span::minutes(minutes), e, Some(&id));
        }
        self.hide(
            now - Span::minutes(30),
            index,
            names::OPERATOR,
            "The operator",
        );
    }

    /// `author` (the operator or the owner) hides the channel at `index`
    /// and says so in it, as `who`. As on the server, its members are
    /// removed and its messages kept.
    fn hide(&mut self, at: Time, index: usize, author: &str, who: &str) -> Vec<Out> {
        let message = self.store(
            at,
            index,
            author.to_string(),
            format!("{who} hid the channel. Its messages are kept."),
        );
        let mut c = self.channels.remove(index);
        c.members.clear();
        c.channel.member_count = 0;
        let hidden = ChannelHidden {
            channel_id: c.channel.id.clone(),
            channel_name: c.channel.name.clone(),
            by: author.to_string(),
            by_operator: author == names::OPERATOR,
        };
        self.hidden.push(c);
        vec![Out::Message(message), Out::ChannelHidden(hidden)]
    }

    /// A random member of a random channel with members leaves it, as an
    /// agent's `leave_channel` does. `None` if no channel has members.
    fn leave(&mut self, e: &dyn Entropy) -> Option<Left> {
        let with_members: Vec<usize> = (0..self.channels.len())
            .filter(|&i| !self.channels[i].members.is_empty())
            .collect();
        if with_members.is_empty() {
            return None;
        }
        let c = &mut self.channels[with_members[e.below(with_members.len())]];
        let agent = c.members.remove(e.below(c.members.len()));
        c.channel.member_count = c.members.len() as i64;
        Some(Left {
            by: agent.clone(),
            agent,
            channel_id: c.channel.id.clone(),
            channel_name: c.channel.name.clone(),
            member_count: c.channel.member_count,
            by_operator: false,
        })
    }

    /// A random channel with an owner, and the owner. `None` if none has one.
    fn owned_channel(&self, e: &dyn Entropy) -> Option<(usize, String)> {
        let owned: Vec<usize> = (0..self.channels.len())
            .filter(|&i| self.channels[i].owner.is_some())
            .collect();
        let index = *owned.get(e.below(owned.len().max(1)))?;
        Some((index, self.channels[index].owner.clone()?))
    }

    fn add_agent(&mut self, now: Time, e: &dyn Entropy) -> String {
        let base = names::agent_handle(e);
        let mut name = base.clone();
        let mut attempt = 0;
        while self.agents.iter().any(|a| a.handle == name) || self.retired_names.contains(&name) {
            attempt += 1;
            name = names::numbered_handle(&base, attempt);
        }
        self.agents.push(Agent {
            handle: name.clone(),
            created_at: clock::to_text(now),
            last_seen: Some(clock::to_text(now)),
            listening: false,
            waiting_channels: Vec::new(),
            run_id: Some(RUN_ID),
            description: match e.below(DESCRIPTIONS.len() + 1) {
                i if i < DESCRIPTIONS.len() => Some(DESCRIPTIONS[i].to_string()),
                _ => None,
            },
            lane: None,
        });
        // A new agent joins an open channel or two.
        let open: Vec<usize> = (0..self.channels.len())
            .filter(|&i| self.channels[i].channel.open)
            .collect();
        for _ in 0..1 + e.below(2) {
            if open.is_empty() {
                break;
            }
            let i = open[e.below(open.len())];
            self.join(i, &name);
        }
        name
    }

    fn create_channel(
        &mut self,
        now: Time,
        e: &dyn Entropy,
        name: String,
        purpose: String,
        open: bool,
        members: Vec<String>,
    ) -> Result<(Channel, Message), SourceError> {
        let name = name.trim();
        let purpose = purpose.trim();
        let limits = LIMITS;
        if name.is_empty()
            || name.chars().any(char::is_control)
            || name.chars().count() > limits.max_channel_name_chars
        {
            return Err(SourceError::Invalid("invalid channel name".into()));
        }
        if purpose.chars().count() > limits.max_purpose_chars {
            return Err(SourceError::Invalid("purpose is too long".into()));
        }
        if !open && members.is_empty() {
            return Err(SourceError::Invalid(
                "a closed channel needs at least one initial member".into(),
            ));
        }
        let mut selected = Vec::new();
        for member in members {
            let member = member.trim();
            if !self.agents.iter().any(|a| a.handle == member) {
                return Err(SourceError::NotFound(format!("no agent {member}")));
            }
            if !selected.iter().any(|m| m == member) {
                selected.push(member.to_string());
            }
        }
        let id = (0..names::MAX_CHANNEL_ID_ATTEMPTS)
            .map(|_| names::channel_id(e, CHANNEL_RANDOM_LEN))
            .find(|id| {
                !self.channels.iter().any(|c| c.channel.id == *id)
                    && !self.messages.iter().any(|m| m.channel_id == *id)
            })
            .ok_or_else(|| SourceError::Failed("could not choose a channel ID".into()))?;
        let channel = Channel {
            id,
            name: name.into(),
            purpose: (!purpose.is_empty()).then(|| purpose.into()),
            open,
            created_by: names::OPERATOR.into(),
            created_at: clock::to_text(now),
            member_count: selected.len() as i64,
            note: String::new(),
            note_edited_by: None,
            note_edited_at: None,
        };
        self.channels.push(SimChannel {
            channel: channel.clone(),
            members: selected,
            owner: None,
        });
        let index = self.channels.len() - 1;
        let notice = self.store(
            now,
            index,
            names::OPERATOR.into(),
            "The operator created the channel.".into(),
        );
        Ok((channel, notice))
    }

    fn add_channel(&mut self, now: Time, e: &dyn Entropy) -> String {
        let (name, purpose, open) = match CHANNELS.get(self.channels_used) {
            Some(&(name, purpose, open)) => (name.to_string(), purpose.to_string(), open),
            None => (
                format!("topic-{}", self.channels_used + 1),
                "Started by the simulated feed".to_string(),
                e.below(4) != 0,
            ),
        };
        self.channels_used += 1;
        let creator = self.agents[e.below(self.agents.len())].handle.clone();
        let id = names::channel_id(e, CHANNEL_RANDOM_LEN);
        self.channels.push(SimChannel {
            channel: Channel {
                note: String::new(),
                note_edited_by: None,
                note_edited_at: None,
                id: id.clone(),
                name,
                purpose: Some(purpose),
                open,
                created_by: creator.clone(),
                created_at: clock::to_text(now),
                member_count: 0,
            },
            members: Vec::new(),
            owner: Some(creator.clone()),
        });
        let index = self.channels.len() - 1;
        self.join(index, &creator);
        for _ in 0..1 + e.below(5) {
            let other = self.agents[e.below(self.agents.len())].handle.clone();
            self.join(index, &other);
        }
        id
    }

    fn join(&mut self, channel: usize, agent: &str) {
        let c = &mut self.channels[channel];
        if !c.members.iter().any(|m| m == agent) {
            c.members.push(agent.to_string());
            c.channel.member_count = c.members.len() as i64;
        }
    }

    /// A channel with members other than quiet-corner, for bursts.
    fn busy_channel(&self, e: &dyn Entropy) -> String {
        let busy = self.channels.len().saturating_sub(1).max(1);
        self.channels[e.below(busy)].channel.id.clone()
    }

    /// A message from a random member of `channel` (or of a random channel).
    fn post(&mut self, at: Time, e: &dyn Entropy, channel: Option<&str>) -> Message {
        let index =
            match channel.and_then(|id| self.channels.iter().position(|c| c.channel.id == id)) {
                Some(i) => i,
                None => e.below(self.channels.len()),
            };
        let members = &self.channels[index].members;
        let agent = if members.is_empty() {
            names::OPERATOR.to_string()
        } else {
            members[e.below(members.len())].clone()
        };
        let body = body(e);
        self.store(at, index, agent, body)
    }

    /// A message from `agent` in the first channel it is in, if any.
    fn post_as(&mut self, at: Time, agent: &str, body: &str) -> Option<Message> {
        let index = self
            .channels
            .iter()
            .position(|c| c.members.iter().any(|m| m == agent))?;
        Some(self.store(at, index, agent.to_string(), body.to_string()))
    }

    /// The human changes a channel's purpose, and the operator says so, as
    /// the server does.
    fn set_purpose(
        &mut self,
        at: Time,
        channel_id: &str,
        purpose: &str,
    ) -> Result<Message, SourceError> {
        let index = self
            .channels
            .iter()
            .position(|c| c.channel.id == channel_id)
            .ok_or_else(|| SourceError::NotFound(format!("no channel {channel_id}")))?;
        let purpose = Some(purpose.trim()).filter(|p| !p.is_empty());
        let body = match purpose {
            Some(p) => format!("The operator changed the purpose to: {p}"),
            None => "The operator cleared the purpose.".to_string(),
        };
        self.channels[index].channel.purpose = purpose.map(str::to_string);
        Ok(self.store(at, index, names::OPERATOR.to_string(), body))
    }

    fn change_channel(
        &mut self,
        at: Time,
        id: &str,
        change: ChannelAction,
    ) -> Result<Vec<Out>, SourceError> {
        let index = self
            .channels
            .iter()
            .position(|c| c.channel.id == id)
            .ok_or_else(|| SourceError::NotFound(format!("no channel {id}")))?;
        let mut event = None;
        let body = match change {
            ChannelAction::SetOpen(open) => {
                if self.channels[index].channel.open == open {
                    return Ok(Vec::new());
                }
                self.channels[index].channel.open = open;
                format!(
                    "The operator made the channel {}.",
                    if open { "open" } else { "closed" }
                )
            }
            ChannelAction::AddMember(name) => {
                let name = name.trim();
                if !self.agents.iter().any(|a| a.handle == name) {
                    return Err(SourceError::NotFound(format!("no agent {name}")));
                }
                if self.channels[index].members.iter().any(|m| m == name) {
                    return Ok(Vec::new());
                }
                self.join(index, name);
                format!("The operator added {name} to the channel.")
            }
            ChannelAction::RemoveMember(name) => {
                let name = name.trim();
                if !self.agents.iter().any(|a| a.handle == name) {
                    return Err(SourceError::NotFound(format!("no agent {name}")));
                }
                let c = &mut self.channels[index];
                if !c.members.iter().any(|m| m == name) {
                    return Ok(Vec::new());
                }
                c.members.retain(|m| m != name);
                c.channel.member_count = c.members.len() as i64;
                event = Some(Out::Left(Left {
                    agent: name.to_string(),
                    channel_id: c.channel.id.clone(),
                    channel_name: c.channel.name.clone(),
                    member_count: c.channel.member_count,
                    by: names::OPERATOR.into(),
                    by_operator: true,
                }));
                format!("The operator removed {name} from the channel.")
            }
            ChannelAction::SetOwner(Some(name)) => {
                let name = name.trim();
                if !self.agents.iter().any(|a| a.handle == name) {
                    return Err(SourceError::NotFound(format!("no agent {name}")));
                }
                let c = &mut self.channels[index];
                if !c.members.iter().any(|m| m == name) {
                    return Err(SourceError::Invalid(format!(
                        "{name} is not a member of {id}; add it first"
                    )));
                }
                if c.owner.as_deref() == Some(name) {
                    return Ok(Vec::new());
                }
                c.owner = Some(name.to_string());
                format!("The operator made {name} the owner of the channel.")
            }
            ChannelAction::SetOwner(None) => {
                let Some(old) = self.channels[index].owner.take() else {
                    return Ok(Vec::new());
                };
                format!(
                    "The operator removed {old} as the owner of the channel. \
                     The channel no longer has an owner."
                )
            }
            ChannelAction::Hide => {
                return Ok(self.hide(at, index, names::OPERATOR, "The operator"));
            }
        };
        let message = self.store(at, index, names::OPERATOR.to_string(), body);
        Ok(std::iter::once(Out::Message(message))
            .chain(event)
            .collect())
    }

    fn hide_agent(&mut self, at: Time, name: &str) -> Result<Vec<Out>, SourceError> {
        if !self.agents.iter().any(|a| a.handle == name) {
            return Err(SourceError::NotFound(format!("no agent {name}")));
        }
        if self.owns_any(name) {
            return Err(SourceError::Invalid(format!(
                "{name} owns a channel; make another member the owner, or clear the \
                 owner, on each channel's page first"
            )));
        }
        let body =
            format!("The operator hid {name} and revoked its access. Its messages are kept.");
        Ok(self.remove_avatar(at, name, names::OPERATOR, &body, "was hidden"))
    }

    /// Removes `name`'s avatar, as the server's hide and unregister do:
    /// `author` posts `body` in each channel `name` was in, and the operator
    /// explains, in each channel `name` owns but had left, that `name`
    /// `what`. Its memberships and ownerships go; its messages stay.
    fn remove_avatar(
        &mut self,
        at: Time,
        name: &str,
        author: &str,
        body: &str,
        what: &str,
    ) -> Vec<Out> {
        let mut out = Vec::new();
        let mut channels = Vec::new();
        for index in 0..self.channels.len() {
            let owned = self.channels[index].owner.as_deref() == Some(name);
            let member = self.channels[index].members.iter().any(|m| m == name);
            let text = match (member, owned) {
                (true, true) => format!("{body} The channel no longer has an owner."),
                (true, false) => body.to_string(),
                (false, true) => format!(
                    "{name} owned the channel and {what}. The channel no longer has an owner."
                ),
                (false, false) => continue,
            };
            let from = if member { author } else { names::OPERATOR };
            out.push(Out::Message(self.store(at, index, from.to_string(), text)));
            let c = &mut self.channels[index];
            if owned {
                c.owner = None;
            }
            if member {
                c.members.retain(|m| m != name);
                c.channel.member_count = c.members.len() as i64;
                channels.push(ChannelMembers {
                    channel_id: c.channel.id.clone(),
                    channel_name: c.channel.name.clone(),
                    member_count: c.channel.member_count,
                });
            }
        }
        self.retired_names.insert(name.to_string());
        self.agents.retain(|a| a.handle != name);
        out.push(Out::AgentHidden(AgentHidden {
            agent: name.to_string(),
            by: author.to_string(),
            by_operator: author == names::OPERATOR,
            channels,
        }));
        out
    }

    fn store(&mut self, at: Time, channel: usize, agent: String, body: String) -> Message {
        let c = &self.channels[channel].channel;
        let message = Message {
            id: self.next_id,
            channel_id: c.id.clone(),
            channel_name: c.name.clone(),
            from_operator: agent == names::OPERATOR,
            agent: agent.clone(),
            body,
            created_at: clock::to_text(at),
            run_id: Some(RUN_ID),
        };
        self.next_id += 1;
        if let Some(a) = self.agents.iter_mut().find(|a| a.handle == agent) {
            a.last_seen = Some(message.created_at.clone());
        }
        self.messages.push(message.clone());
        message
    }

    /// The front page's summary, with recent activity counted since `since`
    /// in stretches of `bucket`.
    fn overview(&self, since: &str, bucket: Duration, now: String) -> Overview {
        let channels = self
            .channels
            .iter()
            .map(|c| self.summary(c, false, since, bucket))
            .collect();
        let mut agents: Vec<AgentSummary> = self
            .agents
            .iter()
            .map(|a| AgentSummary {
                agent: a.clone(),
                channels: self
                    .channels
                    .iter()
                    .filter(|c| c.members.contains(&a.handle))
                    .map(|c| ChannelLink {
                        id: c.channel.id.clone(),
                        name: c.channel.name.clone(),
                    })
                    .collect(),
            })
            .collect();
        agents.sort_by(|a, b| b.agent.last_seen.cmp(&a.agent.last_seen));
        Overview {
            channels,
            agents,
            current_run: self.runs.iter().find(|r| r.id == RUN_ID).cloned(),
            now,
        }
    }

    /// The hidden channels, in name order as the server lists them.
    fn hidden_channels(&self, since: &str, bucket: Duration) -> Vec<ChannelSummary> {
        let mut hidden: Vec<ChannelSummary> = self
            .hidden
            .iter()
            .map(|c| self.summary(c, true, since, bucket))
            .collect();
        hidden.sort_by(|a, b| {
            (&a.channel.name, &a.channel.id).cmp(&(&b.channel.name, &b.channel.id))
        });
        hidden
    }

    /// One channel for the front page, with recent activity counted since
    /// `since` in stretches of `bucket`.
    fn summary(
        &self,
        c: &SimChannel,
        hidden: bool,
        since: &str,
        bucket: Duration,
    ) -> ChannelSummary {
        let mine = self
            .messages
            .iter()
            .filter(|m| m.channel_id == c.channel.id);
        let last = mine.clone().next_back();
        ChannelSummary {
            channel: c.channel.clone(),
            hidden,
            message_count: mine.clone().count() as i64,
            last_message_at: last.map(|m| m.created_at.clone()),
            last_speaker: last.map(|m| m.agent.clone()),
            last_from_operator: last.is_some_and(|m| m.from_operator),
            recent_activity: activity(mine.filter(|m| m.created_at.as_str() >= since), bucket),
        }
    }

    fn channel_view(
        &self,
        id: &str,
        before_id: Option<i64>,
        page_messages: usize,
    ) -> Result<ChannelView, SourceError> {
        let (c, hidden) = match self.channels.iter().find(|c| c.channel.id == id) {
            Some(c) => (c, false),
            None => (
                self.hidden
                    .iter()
                    .find(|c| c.channel.id == id)
                    .ok_or_else(|| SourceError::NotFound(format!("no channel {id}")))?,
                true,
            ),
        };
        let earlier: Vec<&Message> = self
            .messages
            .iter()
            .filter(|m| m.channel_id == id && before_id.is_none_or(|b| m.id < b))
            .collect();
        let skip = earlier.len().saturating_sub(page_messages);
        let messages: Vec<Message> = earlier[skip..].iter().map(|m| (*m).clone()).collect();
        let older_before_id = match skip {
            0 => None,
            _ => messages.first().map(|m| m.id),
        };
        let members: Vec<Agent> = c
            .members
            .iter()
            .filter_map(|m| self.agents.iter().find(|a| &a.handle == m).cloned())
            .collect();
        let latest_messages = members
            .iter()
            .filter_map(|a| {
                self.messages
                    .iter()
                    .rev()
                    .find(|m| m.channel_id == id && m.agent == a.handle)
                    .cloned()
            })
            .collect();
        Ok(ChannelView {
            channel: c.channel.clone(),
            hidden,
            owner: c.owner.clone(),
            members,
            latest_messages,
            messages,
            older_before_id,
        })
    }

    fn owns_any(&self, name: &str) -> bool {
        self.channels
            .iter()
            .any(|c| c.owner.as_deref() == Some(name))
    }

    fn stale_agent(&self, agent: &Agent) -> StaleAgent {
        let links = |keep: &dyn Fn(&SimChannel) -> bool| {
            self.channels
                .iter()
                .filter(|c| keep(c))
                .map(|c| ChannelLink {
                    id: c.channel.id.clone(),
                    name: c.channel.name.clone(),
                })
                .collect()
        };
        StaleAgent {
            channels: links(&|c| c.members.contains(&agent.handle)),
            owns: links(&|c| c.owner.as_deref() == Some(agent.handle.as_str())),
            // As on the agent page: agents that describe themselves keep
            // memory.
            memory_entries: match agent.description {
                Some(_) => PREVIEW_MEMORY.len(),
                None => 0,
            },
            agent: agent.clone(),
        }
    }

    fn agent_view(&self, name: &str) -> Result<AgentView, SourceError> {
        let agent = self
            .agents
            .iter()
            .find(|a| a.handle == name)
            .cloned()
            .ok_or_else(|| SourceError::NotFound(format!("no agent {name}")))?;
        let channels = self
            .channels
            .iter()
            .filter(|c| c.members.iter().any(|m| m == name))
            .map(|c| c.channel.clone())
            .collect();
        let message_count = self.messages.iter().filter(|m| m.agent == name).count() as i64;
        // Agents that describe themselves also keep memory.
        let memory = match agent.description {
            Some(_) => {
                let saved_at = agent
                    .last_seen
                    .clone()
                    .unwrap_or_else(|| agent.created_at.clone());
                PREVIEW_MEMORY
                    .iter()
                    .map(|(name, text)| MemoryEntry {
                        name: name.to_string(),
                        text: text.to_string(),
                        saved_at: saved_at.clone(),
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        Ok(AgentView {
            agent,
            channels,
            message_count,
            memory,
        })
    }
}

/// The time a stale agent's last call is before, as the server sets it.
fn stale_cutoff(now: Time, idle: Duration) -> Result<String, SourceError> {
    if idle.is_zero() {
        return Err(SourceError::Invalid(
            "the idle period must be longer than zero".into(),
        ));
    }
    Span::from_std(idle)
        .ok()
        .and_then(|d| now.checked_sub(d))
        .map(clock::to_text)
        .ok_or_else(|| SourceError::Invalid("the idle period is too long".into()))
}

/// When `agent` last called, or registered if it never has.
fn last_call(agent: &Agent) -> &str {
    agent.last_seen.as_deref().unwrap_or(&agent.created_at)
}

/// Whether `agent` last called before `cutoff` and is not waiting.
fn is_stale(agent: &Agent, cutoff: &str) -> bool {
    last_call(agent) < cutoff && !agent.listening
}

/// `messages` counted in stretches of `bucket`, oldest first.
fn activity<'a>(
    messages: impl Iterator<Item = &'a Message>,
    bucket: Duration,
) -> Vec<ActivityCount> {
    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for m in messages {
        let start =
            clock::floor_text(&m.created_at, bucket).unwrap_or_else(|| m.created_at.clone());
        *counts.entry(start).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(start, count)| ActivityCount { start, count })
        .collect()
}

/// A message body: usually one line, sometimes several long paragraphs.
fn body(e: &dyn Entropy) -> String {
    if e.below(PREVIEW_LONG_MESSAGE_ONE_IN) == 0 {
        let count = 2 + e.below(6);
        (0..count)
            .map(|_| PARAGRAPHS[e.below(PARAGRAPHS.len())])
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        LINES[e.below(LINES.len())].to_string()
    }
}
