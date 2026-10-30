//! The preview: a [`Source`] with simulated data, so the page can run and be
//! worked on with no database, no MCP and no agents.
//!
//! The world starts with agents, open and closed channels (one still empty),
//! and a few hundred messages, some very long. [`Preview::run`] then keeps
//! it moving: steady messages, occasional bursts, new agents and new
//! channels. Everything comes from the injected clock and entropy, so a
//! seeded entropy gives the same world and the same feed every time.

use super::notice::live_event;
use super::source::{
    ActivityCount, Agent, AgentSummary, AgentView, Channel, ChannelAction, ChannelLink,
    ChannelSummary, ChannelView, KeptAgent, KeptReason, LiveEvent, Message, Overview, ServerRun,
    Source, SourceError, StaleAgent, StaleHidden,
};
use crate::clock::{self, Clock};
use crate::config::{
    PREVIEW_AGENTS, PREVIEW_BURST_MAX, PREVIEW_BURST_ONE_IN, PREVIEW_CHANNELS, PREVIEW_HISTORY,
    PREVIEW_LONG_MESSAGE_ONE_IN, PREVIEW_MESSAGES, PREVIEW_NEW_AGENT_ONE_IN,
    PREVIEW_NEW_CHANNEL_ONE_IN, PREVIEW_STALE_HOURS, PreviewSettings, WEB_ACTIVITY_BUCKET,
    WEB_ACTIVITY_WINDOW,
};
use crate::events::{Events, Joined, NewAgent, NewChannel, NewMessage};
use crate::names::{self, Entropy};
use crate::shutdown::Shutdown;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// The simulated server run that is running, which every row belongs to.
/// The runs before it make the upgrade history.
const RUN_ID: i64 = 4;

/// The upgrade history, newest first: this run, started at `start`, and
/// three before it, with values as long as real ones (full commits, a
/// `-dirty` mark, many settings, a long allowed origin) so the history page
/// can be checked at a narrow width.
fn runs(start: DateTime<Utc>, settings: &PreviewSettings) -> Vec<ServerRun> {
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
            chrono::Duration::zero(),
            "preview",
            "preview".to_string(),
            settings_text(&format!(
                "preview steps_per_minute={}",
                settings.steps_per_minute
            )),
        ),
        (
            3,
            chrono::Duration::days(2),
            "0.5.0",
            "4f1c2a9e7b3d5f60a8c1e2d3b4a5968778695a4b".to_string(),
            settings_text("upgrade_backups_kept=5"),
        ),
        (
            2,
            chrono::Duration::days(9),
            "0.4.1",
            "9a8b7c6d5e4f30211203a4b5c6d7e8f9a0b1c2d3-dirty".to_string(),
            settings_text(""),
        ),
        (
            1,
            chrono::Duration::days(23),
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

/// The handoff note simulated agents leave.
const PREVIEW_HANDOFF_NOTE: &str = "Working on the TOML parser rewrite in ~/src/parser.\n\
    Done: tokenizer, tests green.\n\
    Next: error messages; bold-magpie reviews.\n\
    Pushes need my owner's approval.";

/// Characters in a simulated recovery code after its `arc_` prefix.
const PREVIEW_RECOVERY_CODE_CHARS: usize = 26;

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

/// Simulated data for the web page.
pub struct Preview {
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
    settings: PreviewSettings,
    events: Events,
    world: Mutex<World>,
}

/// What one step of the feed did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Happening {
    Message,
    Burst(usize),
    NewAgent(String),
    NewChannel(String),
}

/// What a step publishes, in order, once the world is unlocked.
enum Out {
    Message(Message),
    Agent(NewAgent),
    Channel(NewChannel),
    Joined(Joined),
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
    operator_instructions: Vec<super::source::OperatorInstructionsVersion>,
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
    /// Live feeds end when `shutdown` stops.
    pub fn new(
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
        settings: PreviewSettings,
        shutdown: &Shutdown,
    ) -> Preview {
        let world = World::start(clock.now(), entropy.as_ref(), &settings);
        Preview {
            clock,
            entropy,
            settings,
            events: Events::new().until_stopped(shutdown.clone()),
            world: Mutex::new(world),
        }
    }

    /// Keeps the world moving, one step every `60 / steps_per_minute`
    /// seconds by the clock, until `shutdown` stops.
    pub async fn run(&self, shutdown: Shutdown) {
        let interval = chrono::Duration::milliseconds(
            60_000 / i64::from(self.settings.steps_per_minute.max(1)),
        );
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
        // can create fresh ones without reviving a hidden identity/channel.
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
                name: name.clone(),
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
        } else {
            out.push(Out::Message(world.post(now, e, None)));
            Happening::Message
        };
        drop(world);
        for item in out {
            match item {
                Out::Message(m) => self.publish_message(m),
                Out::Agent(a) => self.events.publish_agent(a),
                Out::Channel(c) => self.events.publish_channel(c),
                Out::Joined(j) => self.events.publish_joined(j),
            }
        }
        happening
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

    /// The start of `WEB_ACTIVITY_WINDOW`, as a stored time.
    fn activity_since(&self) -> String {
        clock::to_text(
            self.clock.now()
                - chrono::Duration::from_std(WEB_ACTIVITY_WINDOW)
                    .unwrap_or(chrono::Duration::minutes(5)),
        )
    }

    fn world(&self) -> MutexGuard<'_, World> {
        // A poisoned lock means a step panicked part way; the world is still
        // readable, and it is only simulated data.
        self.world.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Source for Preview {
    fn overview(&self) -> BoxFuture<'_, Result<Overview, SourceError>> {
        let now = self.clock.now();
        let world = self.world();
        let overview = world.overview(&self.activity_since(), clock::to_text(now));
        Box::pin(async move { Ok(overview) })
    }

    fn hidden_channels(&self) -> BoxFuture<'_, Result<Vec<ChannelSummary>, SourceError>> {
        let hidden = self.world().hidden_channels(&self.activity_since());
        Box::pin(async move { Ok(hidden) })
    }

    fn server_runs(&self) -> BoxFuture<'_, Result<Vec<ServerRun>, SourceError>> {
        let runs = self.world().runs.clone();
        Box::pin(async move { Ok(runs) })
    }

    fn channel(
        &self,
        id: String,
        before_id: Option<i64>,
    ) -> BoxFuture<'_, Result<ChannelView, SourceError>> {
        let world = self.world();
        let view = world.channel_view(&id, before_id, self.settings.page_messages);
        Box::pin(async move { view })
    }

    fn agent(&self, name: String) -> BoxFuture<'_, Result<AgentView, SourceError>> {
        let world = self.world();
        let view = world.agent_view(&name);
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
        let result = result.map(|messages| {
            for message in messages {
                self.publish_message(message);
            }
        });
        Box::pin(async move { result })
    }

    fn hide_agent(&self, name: String) -> BoxFuture<'_, Result<(), SourceError>> {
        let result = self.world().hide_agent(self.clock.now(), &name);
        let result = result.map(|messages| {
            for message in messages {
                self.publish_message(message);
            }
        });
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
            let mut notices = Vec::new();
            for name in chosen {
                if outcome.hidden.contains(&name) || outcome.kept.iter().any(|k| k.name == name) {
                    continue;
                }
                let reason = match world.agents.iter().find(|a| a.name == name) {
                    None => Some(KeptReason::Gone),
                    Some(a) if !is_stale(a, &cutoff) => Some(KeptReason::Active),
                    Some(_) if world.owns_any(&name) => Some(KeptReason::Owner),
                    Some(_) => None,
                };
                match reason {
                    Some(reason) => outcome.kept.push(KeptAgent { name, reason }),
                    None => {
                        notices.extend(world.hide_agent(now, &name)?);
                        outcome.hidden.push(name);
                    }
                }
            }
            Ok((outcome, notices))
        });
        let result = result.map(|(outcome, notices)| {
            for message in notices {
                self.publish_message(message);
            }
            outcome
        });
        Box::pin(async move { result })
    }

    fn new_recovery_code(&self, name: String) -> BoxFuture<'_, Result<String, SourceError>> {
        let known = self.world().agents.iter().any(|a| a.name == name);
        let code = match known {
            true => Ok(format!(
                "arc_{}",
                names::random_chars(self.entropy.as_ref(), PREVIEW_RECOVERY_CODE_CHARS)
            )),
            false => Err(SourceError::NotFound(format!("no agent {name}"))),
        };
        Box::pin(async move { code })
    }

    fn operator_instructions(
        &self,
    ) -> BoxFuture<'_, Result<super::source::OperatorInstructionsView, SourceError>> {
        let view = super::source::OperatorInstructionsView {
            history: self.world().operator_instructions.clone(),
            max_chars: crate::config::Limits::DEFAULT.max_operator_instructions_chars,
        };
        Box::pin(async move { Ok(view) })
    }
    fn save_operator_instructions(&self, text: String) -> BoxFuture<'_, Result<(), SourceError>> {
        let max = crate::config::Limits::DEFAULT.max_operator_instructions_chars;
        if text.chars().count() > max {
            return Box::pin(async move {
                Err(SourceError::Invalid(format!(
                    "operator instructions must be at most {max} characters"
                )))
            });
        }
        let mut world = self.world();
        let version = world
            .operator_instructions
            .first()
            .map_or(1, |v| v.version + 1);
        world.operator_instructions.insert(
            0,
            super::source::OperatorInstructionsVersion {
                version,
                text,
                saved_at: clock::to_text(self.clock.now()),
                run_id: RUN_ID,
            },
        );
        Box::pin(async { Ok(()) })
    }

    fn live(&self) -> BoxStream<'static, LiveEvent> {
        self.events
            .subscribe()
            .into_stream()
            .map(live_event)
            .boxed()
    }

    fn page_messages(&self) -> usize {
        self.settings.page_messages
    }
}

impl World {
    fn start(now: DateTime<Utc>, e: &dyn Entropy, settings: &PreviewSettings) -> World {
        let history =
            chrono::Duration::from_std(PREVIEW_HISTORY).unwrap_or(chrono::Duration::hours(6));
        let start = now - history;
        let mut world = World {
            retired_names: std::collections::HashSet::new(),
            operator_instructions: Vec::new(),
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
            agent.last_seen = Some(clock::to_text(now - chrono::Duration::minutes(minutes)));
        }
        // Agents that registered and went quiet, the first owning the empty
        // channel, so the stale agents page has owners and non-owners.
        for (k, &hours) in PREVIEW_STALE_HOURS.iter().enumerate() {
            let name = world.add_agent(now - chrono::Duration::hours(hours), e);
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
    fn add_hidden_channel(&mut self, now: DateTime<Utc>, e: &dyn Entropy) {
        let id = self.add_channel(now - chrono::Duration::hours(2), e);
        let index = self.channels.len() - 1;
        let (name, purpose) = HIDDEN_CHANNEL;
        self.channels[index].channel.name = name.into();
        self.channels[index].channel.purpose = Some(purpose.into());
        for minutes in [90, 80, 70, 60, 50, 40] {
            self.post(now - chrono::Duration::minutes(minutes), e, Some(&id));
        }
        self.hide(now - chrono::Duration::minutes(30), index);
    }

    /// The operator hides the channel at `index` and says so in it. As on
    /// the server, its members are removed and its messages kept.
    fn hide(&mut self, at: DateTime<Utc>, index: usize) -> Message {
        let message = self.store(
            at,
            index,
            names::OPERATOR.to_string(),
            "The operator hid the channel. Its messages are kept.".into(),
        );
        let mut c = self.channels.remove(index);
        c.members.clear();
        c.channel.member_count = 0;
        self.hidden.push(c);
        message
    }

    fn add_agent(&mut self, now: DateTime<Utc>, e: &dyn Entropy) -> String {
        let base = names::agent_name(e);
        let mut name = base.clone();
        let mut attempt = 0;
        while self.agents.iter().any(|a| a.name == name) || self.retired_names.contains(&name) {
            attempt += 1;
            name = names::numbered_name(&base, attempt);
        }
        self.agents.push(Agent {
            name: name.clone(),
            created_at: clock::to_text(now),
            last_seen: Some(clock::to_text(now)),
            listening: false,
            waiting_channels: Vec::new(),
            run_id: Some(RUN_ID),
            description: match e.below(DESCRIPTIONS.len() + 1) {
                i if i < DESCRIPTIONS.len() => Some(DESCRIPTIONS[i].to_string()),
                _ => None,
            },
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
        now: DateTime<Utc>,
        e: &dyn Entropy,
        name: String,
        purpose: String,
        open: bool,
        members: Vec<String>,
    ) -> Result<(Channel, Message), SourceError> {
        let name = name.trim();
        let purpose = purpose.trim();
        let limits = crate::config::Limits::DEFAULT;
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
            if !self.agents.iter().any(|a| a.name == member) {
                return Err(SourceError::NotFound(format!("no agent {member}")));
            }
            if !selected.iter().any(|m| m == member) {
                selected.push(member.to_string());
            }
        }
        let id = (0..crate::config::MAX_CHANNEL_ID_ATTEMPTS)
            .map(|_| names::channel_id(e))
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

    fn add_channel(&mut self, now: DateTime<Utc>, e: &dyn Entropy) -> String {
        let (name, purpose, open) = match CHANNELS.get(self.channels_used) {
            Some(&(name, purpose, open)) => (name.to_string(), purpose.to_string(), open),
            None => (
                format!("topic-{}", self.channels_used + 1),
                "Started by the simulated feed".to_string(),
                e.below(4) != 0,
            ),
        };
        self.channels_used += 1;
        let creator = self.agents[e.below(self.agents.len())].name.clone();
        let id = names::channel_id(e);
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
            let other = self.agents[e.below(self.agents.len())].name.clone();
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
    fn post(&mut self, at: DateTime<Utc>, e: &dyn Entropy, channel: Option<&str>) -> Message {
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
    fn post_as(&mut self, at: DateTime<Utc>, agent: &str, body: &str) -> Option<Message> {
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
        at: DateTime<Utc>,
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
        at: DateTime<Utc>,
        id: &str,
        change: ChannelAction,
    ) -> Result<Vec<Message>, SourceError> {
        let index = self
            .channels
            .iter()
            .position(|c| c.channel.id == id)
            .ok_or_else(|| SourceError::NotFound(format!("no channel {id}")))?;
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
                if !self.agents.iter().any(|a| a.name == name) {
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
                if !self.agents.iter().any(|a| a.name == name) {
                    return Err(SourceError::NotFound(format!("no agent {name}")));
                }
                let c = &mut self.channels[index];
                if !c.members.iter().any(|m| m == name) {
                    return Ok(Vec::new());
                }
                c.members.retain(|m| m != name);
                c.channel.member_count = c.members.len() as i64;
                format!("The operator removed {name} from the channel.")
            }
            ChannelAction::SetOwner(Some(name)) => {
                let name = name.trim();
                if !self.agents.iter().any(|a| a.name == name) {
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
            ChannelAction::Hide => return Ok(vec![self.hide(at, index)]),
        };
        Ok(vec![self.store(
            at,
            index,
            names::OPERATOR.to_string(),
            body,
        )])
    }

    fn hide_agent(&mut self, at: DateTime<Utc>, name: &str) -> Result<Vec<Message>, SourceError> {
        if !self.agents.iter().any(|a| a.name == name) {
            return Err(SourceError::NotFound(format!("no agent {name}")));
        }
        if self.owns_any(name) {
            return Err(SourceError::Invalid(format!(
                "{name} owns a channel; make another member the owner, or clear the \
                 owner, on each channel's page first"
            )));
        }
        let mut notices = Vec::new();
        for index in 0..self.channels.len() {
            let c = &mut self.channels[index];
            if !c.members.iter().any(|m| m == name) {
                continue;
            }
            let body =
                format!("The operator hid {name} and revoked its access. Its messages are kept.");
            c.members.retain(|m| m != name);
            c.channel.member_count = c.members.len() as i64;
            notices.push(self.store(at, index, names::OPERATOR.to_string(), body));
        }
        self.retired_names.insert(name.to_string());
        self.agents.retain(|a| a.name != name);
        Ok(notices)
    }

    fn store(&mut self, at: DateTime<Utc>, channel: usize, agent: String, body: String) -> Message {
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
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == agent) {
            a.last_seen = Some(message.created_at.clone());
        }
        self.messages.push(message.clone());
        message
    }

    /// The front page's summary, with recent activity counted since `since`.
    fn overview(&self, since: &str, now: String) -> Overview {
        let channels = self
            .channels
            .iter()
            .map(|c| self.summary(c, false, since))
            .collect();
        let mut agents: Vec<AgentSummary> = self
            .agents
            .iter()
            .map(|a| AgentSummary {
                agent: a.clone(),
                channels: self
                    .channels
                    .iter()
                    .filter(|c| c.members.contains(&a.name))
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
    fn hidden_channels(&self, since: &str) -> Vec<ChannelSummary> {
        let mut hidden: Vec<ChannelSummary> = self
            .hidden
            .iter()
            .map(|c| self.summary(c, true, since))
            .collect();
        hidden.sort_by(|a, b| {
            (&a.channel.name, &a.channel.id).cmp(&(&b.channel.name, &b.channel.id))
        });
        hidden
    }

    /// One channel for the front page, with recent activity counted since
    /// `since`.
    fn summary(&self, c: &SimChannel, hidden: bool, since: &str) -> ChannelSummary {
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
            recent_activity: activity(mine.filter(|m| m.created_at.as_str() >= since)),
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
        Ok(ChannelView {
            channel: c.channel.clone(),
            hidden,
            owner: c.owner.clone(),
            members: c
                .members
                .iter()
                .filter_map(|m| self.agents.iter().find(|a| &a.name == m).cloned())
                .collect(),
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
            channels: links(&|c| c.members.contains(&agent.name)),
            owns: links(&|c| c.owner.as_deref() == Some(agent.name.as_str())),
            // As on the agent page: agents that describe themselves leave a
            // handoff note.
            has_handoff_note: agent.description.is_some(),
            agent: agent.clone(),
        }
    }

    fn agent_view(&self, name: &str) -> Result<AgentView, SourceError> {
        let agent = self
            .agents
            .iter()
            .find(|a| a.name == name)
            .cloned()
            .ok_or_else(|| SourceError::NotFound(format!("no agent {name}")))?;
        let channels = self
            .channels
            .iter()
            .filter(|c| c.members.iter().any(|m| m == name))
            .map(|c| c.channel.clone())
            .collect();
        let message_count = self.messages.iter().filter(|m| m.agent == name).count() as i64;
        // Agents that describe themselves also leave a handoff note.
        let (handoff_note, handoff_note_set_at) = match agent.description {
            Some(_) => (PREVIEW_HANDOFF_NOTE.to_string(), agent.last_seen.clone()),
            None => (String::new(), None),
        };
        Ok(AgentView {
            agent,
            channels,
            message_count,
            handoff_note,
            handoff_note_set_at,
        })
    }
}

/// The time a stale agent's last call is before, as the server sets it.
fn stale_cutoff(now: DateTime<Utc>, idle: Duration) -> Result<String, SourceError> {
    if idle.is_zero() {
        return Err(SourceError::Invalid(
            "the idle period must be longer than zero".into(),
        ));
    }
    chrono::Duration::from_std(idle)
        .ok()
        .and_then(|d| now.checked_sub_signed(d))
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

/// `messages` counted in stretches of `WEB_ACTIVITY_BUCKET`, oldest first.
fn activity<'a>(messages: impl Iterator<Item = &'a Message>) -> Vec<ActivityCount> {
    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for m in messages {
        let start = clock::floor_text(&m.created_at, WEB_ACTIVITY_BUCKET)
            .unwrap_or_else(|| m.created_at.clone());
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
