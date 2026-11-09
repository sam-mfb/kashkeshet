//! The server's side of the web contract: [`AdmissionGate`] is the gate
//! over [`WebAdmission`] and [`Identify`], which decides who each viewer is
//! ([`WebViewer`]: a swarm and an operator); [`RulesSources`] gives each
//! viewer a [`RulesSource`], which reads through the rules in the viewer's
//! swarm, as the viewer's operator. They only translate between the rules',
//! admission's and identity's types and the web's: who someone is and what
//! they may do is decided by `Identify` alone.

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use kkt_admission::{self as admission, Credentials, Decision, WebAdmission};
use kkt_config::{MAX_LANE_DESCRIPTION_CHARS, MAX_LANE_NAME_CHARS};
use kkt_identity::Identify;
use kkt_names as names;
use kkt_rules::{
    self as rules, Actor, AgentInfo, ChannelInfo, Code, Operator, Refusal, RuleError, Rules,
    ServerRunInfo, SwarmId,
};
use kkt_web::live_event;
use kkt_web::{
    ActivityCount, Admit, Agent, AgentLane, AgentSummary, AgentView, Channel, ChannelAction,
    ChannelLink, ChannelSummary, ChannelView, Gate, KeptAgent, KeptReason, LaneLimits, LaneRetired,
    LaneSave, LaneSource, LaneSummary, LaneVersion, LaneView, LanesView, LiveEvent, MemoryEntry,
    Message, Overview, ServerAnnouncementVersion, ServerAnnouncementView, ServerRun, SignIn,
    Source, SourceError, Sources, StaleAgent, StaleHidden,
};
use std::sync::Arc;
use std::time::Duration;

/// Who is viewing the web page, as [`AdmissionGate`] decides it through
/// [`Identify`]: the swarm the viewer sees, and the operator they act as.
/// kkt-web passes it on without looking inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewer {
    pub swarm: SwarmId,
    pub operator: Operator,
}

/// Each viewer's [`RulesSource`]: the rules in the viewer's swarm
/// ([`Rules::in_swarm`]), as the viewer's operator.
pub struct RulesSources {
    rules: Rules,
}

impl RulesSources {
    pub fn new(rules: Rules) -> RulesSources {
        RulesSources { rules }
    }
}

impl Sources for RulesSources {
    type Viewer = WebViewer;

    fn source(&self, viewer: &WebViewer) -> Arc<dyn Source> {
        Arc::new(RulesSource::new(
            self.rules.in_swarm(viewer.swarm.clone()),
            viewer.operator.clone(),
        ))
    }
}

/// The pages' data for one viewer, read through `rules` (in the viewer's
/// swarm) as the viewer's operator. What the operator may read and change
/// is the rules' to check.
pub struct RulesSource {
    rules: Rules,
    operator: Operator,
}

impl RulesSource {
    pub fn new(rules: Rules, operator: Operator) -> RulesSource {
        RulesSource { rules, operator }
    }

    /// The viewer's operator.
    fn operator(&self) -> Operator {
        self.operator.clone()
    }

    /// Who makes a change on the web page: [`RulesSource::operator`].
    fn actor(&self) -> Actor {
        Actor::Operator(self.operator())
    }

    fn lane_limits(&self) -> LaneLimits {
        LaneLimits {
            max_name_chars: MAX_LANE_NAME_CHARS,
            max_description_chars: MAX_LANE_DESCRIPTION_CHARS,
            max_text_chars: self.rules.limits().max_lane_chars,
        }
    }
}

impl Source for RulesSource {
    fn overview(&self) -> BoxFuture<'_, Result<Overview, SourceError>> {
        Box::pin(async move {
            let o = self
                .rules
                .overview(&self.operator())
                .await
                .map_err(source_error)?;
            Ok(Overview {
                channels: o.channels.into_iter().map(|c| summary(c, false)).collect(),
                agents: o
                    .agents
                    .into_iter()
                    .map(|a| AgentSummary {
                        agent: agent(a.info),
                        channels: a
                            .channels
                            .into_iter()
                            .map(|c| ChannelLink {
                                id: c.id,
                                name: c.name,
                            })
                            .collect(),
                    })
                    .collect(),
                current_run: o.current_run.map(run),
                now: o.now,
            })
        })
    }

    fn hidden_channels(&self) -> BoxFuture<'_, Result<Vec<ChannelSummary>, SourceError>> {
        Box::pin(async move {
            let hidden = self
                .rules
                .hidden_channels(&self.operator())
                .await
                .map_err(source_error)?;
            Ok(hidden.into_iter().map(|c| summary(c, true)).collect())
        })
    }

    fn server_runs(&self) -> BoxFuture<'_, Result<Vec<ServerRun>, SourceError>> {
        Box::pin(async move {
            let runs = self
                .rules
                .server_runs(&self.operator())
                .await
                .map_err(source_error)?;
            Ok(runs.into_iter().map(run).collect())
        })
    }

    fn channel(
        &self,
        id: String,
        before_id: Option<i64>,
    ) -> BoxFuture<'_, Result<ChannelView, SourceError>> {
        Box::pin(async move {
            let v = self
                .rules
                .channel_view(&self.operator(), id, before_id)
                .await
                .map_err(source_error)?;
            Ok(ChannelView {
                hidden: v.hidden,
                owner: v.channel.owner.clone(),
                channel: channel(v.channel),
                members: v.members.into_iter().map(agent).collect(),
                latest_messages: v.latest_messages.into_iter().map(message).collect(),
                messages: v.messages.into_iter().map(message).collect(),
                older_before_id: v.older_before_id,
            })
        })
    }

    fn agent(&self, handle: String) -> BoxFuture<'_, Result<AgentView, SourceError>> {
        Box::pin(async move {
            let v = self
                .rules
                .agent_view(&self.operator(), handle)
                .await
                .map_err(source_error)?;
            Ok(AgentView {
                agent: agent(v.agent),
                channels: v.channels.into_iter().map(channel).collect(),
                message_count: v.message_count,
                memory: v
                    .memory
                    .into_iter()
                    .map(|e| MemoryEntry {
                        name: e.name,
                        text: e.text,
                        saved_at: e.saved_at,
                    })
                    .collect(),
            })
        })
    }

    fn new_recovery_code(&self, handle: String) -> BoxFuture<'_, Result<String, SourceError>> {
        Box::pin(async move {
            self.rules
                .new_recovery_code(&self.operator(), handle)
                .await
                .map_err(source_error)
        })
    }

    fn create_channel(
        &self,
        name: String,
        purpose: String,
        open: bool,
        members: Vec<String>,
    ) -> BoxFuture<'_, Result<String, SourceError>> {
        Box::pin(async move {
            self.rules
                .create_channel_as_operator(
                    self.actor(),
                    rules::NewChannel {
                        name,
                        purpose: Some(purpose),
                        open,
                        members,
                    },
                )
                .await
                .map(|c| c.id)
                .map_err(source_error)
        })
    }

    fn set_purpose(
        &self,
        channel_id: String,
        purpose: String,
    ) -> BoxFuture<'_, Result<(), SourceError>> {
        Box::pin(async move {
            self.rules
                .set_purpose(self.actor(), channel_id, purpose)
                .await
                .map(|_| ())
                .map_err(source_error)
        })
    }

    fn change_channel(
        &self,
        channel_id: String,
        change: ChannelAction,
    ) -> BoxFuture<'_, Result<(), SourceError>> {
        let change = match change {
            ChannelAction::SetOpen(open) => kkt_rules::ChannelChange::SetOpen(open),
            ChannelAction::AddMember(handle) => kkt_rules::ChannelChange::AddMember(handle),
            ChannelAction::RemoveMember(handle) => kkt_rules::ChannelChange::RemoveMember(handle),
            ChannelAction::SetOwner(handle) => kkt_rules::ChannelChange::SetOwner(handle),
            ChannelAction::Hide => kkt_rules::ChannelChange::Hide,
        };
        Box::pin(async move {
            self.rules
                .change_channel(self.actor(), channel_id, change)
                .await
                .map_err(source_error)
        })
    }

    fn hide_agent(&self, handle: String) -> BoxFuture<'_, Result<(), SourceError>> {
        Box::pin(async move {
            self.rules
                .hide_agent(self.actor(), handle)
                .await
                .map_err(source_error)
        })
    }

    fn stale_agents(&self, idle: Duration) -> BoxFuture<'_, Result<Vec<StaleAgent>, SourceError>> {
        Box::pin(async move {
            let stale = self
                .rules
                .stale_agents(&self.operator(), idle)
                .await
                .map_err(source_error)?;
            Ok(stale
                .into_iter()
                .map(|s| StaleAgent {
                    agent: agent(s.info),
                    channels: s.channels.into_iter().map(link).collect(),
                    memory_entries: s.memory_entries,
                    owns: s.owns.into_iter().map(link).collect(),
                })
                .collect())
        })
    }

    fn hide_stale_agents(
        &self,
        idle: Duration,
        chosen: Vec<String>,
    ) -> BoxFuture<'_, Result<StaleHidden, SourceError>> {
        Box::pin(async move {
            let outcome = self
                .rules
                .hide_stale_agents(self.actor(), idle, chosen)
                .await
                .map_err(source_error)?;
            Ok(StaleHidden {
                hidden: outcome.hidden,
                kept: outcome
                    .kept
                    .into_iter()
                    .map(|k| KeptAgent {
                        handle: k.handle,
                        reason: match k.reason {
                            rules::KeptReason::Gone => KeptReason::Gone,
                            rules::KeptReason::Active => KeptReason::Active,
                            rules::KeptReason::Owner => KeptReason::Owner,
                        },
                    })
                    .collect(),
            })
        })
    }

    fn server_announcement(&self) -> BoxFuture<'_, Result<ServerAnnouncementView, SourceError>> {
        Box::pin(async move {
            let history = self
                .rules
                .server_announcement_history(&self.operator())
                .await
                .map_err(source_error)?;
            Ok(ServerAnnouncementView {
                max_chars: self.rules.limits().max_server_announcement_chars,
                history: history
                    .into_iter()
                    .map(|v| ServerAnnouncementVersion {
                        version: v.version,
                        text: v.text,
                        saved_at: v.saved_at,
                        run_id: v.run_id,
                        saved_by_admin: v.saved_by_admin,
                    })
                    .collect(),
            })
        })
    }
    fn save_server_announcement(&self, text: String) -> BoxFuture<'_, Result<(), SourceError>> {
        Box::pin(async move {
            self.rules
                .save_server_announcement(self.actor(), text)
                .await
                .map(|_| ())
                .map_err(source_error)
        })
    }

    fn lanes(&self) -> BoxFuture<'_, Result<LanesView, SourceError>> {
        Box::pin(async move {
            let lanes = self
                .rules
                .lanes(&self.operator())
                .await
                .map_err(source_error)?;
            Ok(LanesView {
                lanes: lanes
                    .into_iter()
                    .map(|l| LaneSummary {
                        name: l.name,
                        created_at: l.created_at,
                        retired: l.retired.map(lane_retired),
                        current: lane_version(l.current),
                    })
                    .collect(),
                limits: self.lane_limits(),
            })
        })
    }

    fn lane(&self, name: String) -> BoxFuture<'_, Result<LaneView, SourceError>> {
        Box::pin(async move {
            let h = self
                .rules
                .lane_history(&self.operator(), name)
                .await
                .map_err(source_error)?;
            Ok(LaneView {
                name: h.lane.name,
                created_at: h.lane.created_at,
                retired: h.lane.retired.map(lane_retired),
                history: h.versions.into_iter().map(lane_version).collect(),
                limits: self.lane_limits(),
            })
        })
    }

    fn save_lane(&self, save: LaneSave, create: bool) -> BoxFuture<'_, Result<(), SourceError>> {
        let save = rules::LaneSave {
            name: save.name,
            description: save.description,
            text: save.text,
        };
        Box::pin(async move {
            let saved = match create {
                true => self.rules.create_lane(self.actor(), save).await,
                false => self.rules.update_lane(self.actor(), save).await,
            };
            saved.map(|_| ()).map_err(source_error)
        })
    }

    fn retire_lane(&self, name: String) -> BoxFuture<'_, Result<(), SourceError>> {
        Box::pin(async move {
            self.rules
                .retire_lane(self.actor(), name)
                .await
                .map(|_| ())
                .map_err(source_error)
        })
    }

    fn live(&self) -> BoxStream<'static, LiveEvent> {
        self.rules.subscribe().into_stream().map(live_event).boxed()
    }

    fn page_messages(&self) -> usize {
        self.rules.limits().web_page_messages
    }

    fn member_excerpt_chars(&self) -> usize {
        self.rules.limits().web_member_excerpt_chars
    }
}

/// The web page's gate over the server's [`WebAdmission`] and
/// [`Identify`]. Admission says whether the viewer's session (or, with no
/// sign-in, anyone) is let in and as which principal; `Identify` says which
/// operator that principal is, and in which swarm, exactly as it does for
/// the admin endpoint. The gate only translates their answers into the
/// web's.
pub struct AdmissionGate {
    admission: WebAdmission,
    identify: Arc<dyn Identify>,
}

impl AdmissionGate {
    pub fn new(admission: WebAdmission, identify: Arc<dyn Identify>) -> AdmissionGate {
        AdmissionGate {
            admission,
            identify,
        }
    }
}

impl Gate for AdmissionGate {
    type Viewer = WebViewer;

    fn sign_in_required(&self) -> bool {
        WebAdmission::sign_in_required(&self.admission)
    }

    fn admits<'a>(&'a self, session_id: Option<&'a str>) -> BoxFuture<'a, Admit<WebViewer>> {
        Box::pin(async move {
            let principal = match self.admission.decide(session_id) {
                Decision::Admitted(principal) => principal,
                Decision::Refused(refusal) => return Admit::No(refusal.to_string()),
            };
            match self.identify.operator(&principal).await {
                Ok(who) => Admit::Yes(WebViewer {
                    swarm: who.swarm,
                    operator: who.actor,
                }),
                Err(e) => Admit::No(word(e).message),
            }
        })
    }

    fn sign_in(&self, key: &str) -> SignIn {
        match WebAdmission::sign_in(&self.admission, &Credentials::from_key(key)) {
            admission::SignIn::Started {
                session_id,
                lifetime,
            } => SignIn::Started {
                session_id,
                lifetime,
            },
            admission::SignIn::Refused(refusal) => SignIn::Refused(refusal.to_string()),
        }
    }

    fn sign_out(&self, session_id: &str) {
        WebAdmission::sign_out(&self.admission, session_id)
    }

    fn form_token(&self, session_id: Option<&str>) -> Option<String> {
        WebAdmission::form_token(&self.admission, session_id)
    }

    fn form_token_matches(&self, session_id: Option<&str>, presented: &str) -> bool {
        WebAdmission::form_token_matches(&self.admission, session_id, presented)
    }
}

fn source_error(e: RuleError) -> SourceError {
    let e = word(e);
    match e.code {
        Code::NotFound => SourceError::NotFound(e.message),
        Code::InvalidArgument => SourceError::Invalid(e.message),
        _ => SourceError::Failed(e.to_string()),
    }
}

fn channel(c: ChannelInfo) -> Channel {
    Channel {
        note: c.note.text,
        note_edited_by: c.note.edited_by,
        note_edited_at: c.note.edited_at,
        id: c.id,
        name: c.name,
        purpose: c.purpose,
        open: c.open,
        created_by: c.created_by,
        created_at: c.created_at,
        member_count: c.member_count,
    }
}

fn summary(c: rules::ChannelActivity, hidden: bool) -> ChannelSummary {
    ChannelSummary {
        channel: channel(c.info),
        hidden,
        message_count: c.message_count,
        last_message_at: c.last_message_at,
        last_from_operator: c.last_speaker.as_deref().is_some_and(names::is_operator),
        last_speaker: c.last_speaker,
        recent_activity: c
            .recent_activity
            .into_iter()
            .map(|a| ActivityCount {
                start: a.start,
                count: a.count,
            })
            .collect(),
    }
}

fn agent(a: AgentInfo) -> Agent {
    Agent {
        handle: a.handle,
        created_at: a.created_at,
        last_seen: a.last_seen,
        listening: a.listening,
        waiting_channels: a.waiting_channels,
        run_id: a.run_id,
        description: a.description,
        lane: a.lane.map(|l| AgentLane {
            name: l.name,
            version: l.version,
            current_version: l.current_version,
        }),
    }
}

fn link(c: rules::ChannelRef) -> ChannelLink {
    ChannelLink {
        id: c.id,
        name: c.name,
    }
}

fn message(m: rules::Message) -> Message {
    Message {
        id: m.id,
        channel_id: m.channel_id,
        channel_name: m.channel_name,
        from_operator: names::is_operator(&m.agent),
        agent: m.agent,
        body: m.body,
        created_at: m.created_at,
        run_id: m.run_id,
    }
}

fn lane_version(v: rules::LaneVersion) -> LaneVersion {
    LaneVersion {
        version: v.version,
        description: v.description,
        text: v.text,
        saved_at: v.saved_at,
        run_id: v.run_id,
        saved_by_admin: v.saved_by_admin,
        source: v.source.map(|s| LaneSource {
            folder: s.folder,
            commit: s.commit,
        }),
    }
}

fn lane_retired(r: rules::LaneRetired) -> LaneRetired {
    LaneRetired {
        at: r.at,
        by_admin: r.by_admin,
    }
}

fn run(r: ServerRunInfo) -> ServerRun {
    ServerRun {
        id: r.id,
        started_at: r.started_at,
        version: r.version,
        git_commit: r.git_commit,
        settings: r.settings,
        instructions_sha256: r.instructions_sha256,
    }
}

#[cfg(test)]
mod tests;

/// `e`, with the advice the web page gives for the refusals it can act on:
/// where to pass on or clear an owner. Other errors are as `rules` gave
/// them.
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
