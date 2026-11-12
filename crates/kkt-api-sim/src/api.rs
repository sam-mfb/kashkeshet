//! The preview as the kkt-api crates' operations: the parts the page uses
//! answer from the simulated world, in kkt-api's records; every other
//! operation is refused. Each swarm is a world of its own
//! ([`Preview::in_swarm`]).

use super::records::{self, SourceError};
use super::{LIMITS, Preview};
use futures::future::BoxFuture;
use kkt_api_agent::{
    Actor, Agent, AgentInfo, AgentView, Agents, Answer, Api, ChannelInfo, ChannelNote,
    ChannelQuery, ChannelRef, Channels, ClientInfo, Code, HeldLane, Lane, LaneRetired, LaneSource,
    LaneTaking, LaneVersion, Limits, Members, Memory, MemoryEntry, MemoryList, MemorySaved,
    Message, MessagePage, Messages, NewChannel, Operator, Page, ReadQuery, Reclaimed, Registration,
    RuleError, SearchHit, SearchQuery, ServerAnnouncement, SwarmId, WaitQuery, WaitResult,
};
use kkt_api_operator::{
    ActivityCount, AgentActivity, Changes, ChannelActivity, ChannelChange, ChannelView, Kept,
    KeptReason, LaneHistory, LaneSave, NextNotice, Notice, Notices, OperatorApi, Overview, Server,
    ServerRunInfo, StaleAgent, StaleHidden, Views,
};
use kkt_events::Subscription;
use std::sync::Arc;
use std::time::Duration;

/// The answer to an operation the page never asks of the preview.
fn refused<'a, T: Send + 'a>() -> Answer<'a, T> {
    Box::pin(std::future::ready(Err(RuleError::new(
        Code::Internal,
        "the preview does not do this",
    ))))
}

/// `answer`, in kkt-api's terms: each record by `convert`, each error as
/// the rules would give it.
fn answer<'a, T: Send + 'a, U: 'a>(
    answer: BoxFuture<'a, Result<T, SourceError>>,
    convert: impl FnOnce(T) -> U + Send + 'a,
) -> Answer<'a, U> {
    Box::pin(async move { answer.await.map(convert).map_err(rule_error) })
}

fn rule_error(e: SourceError) -> RuleError {
    match e {
        SourceError::NotFound(message) => RuleError::new(Code::NotFound, message),
        SourceError::Invalid(message) => RuleError::invalid(message),
        SourceError::Failed(message) => RuleError::new(Code::Internal, message),
    }
}

fn channel(c: records::Channel, owner: Option<String>) -> ChannelInfo {
    ChannelInfo {
        note: ChannelNote {
            channel_id: c.id.clone(),
            text: c.note,
            edited_by: c.note_edited_by,
            edited_at: c.note_edited_at,
        },
        id: c.id,
        name: c.name,
        purpose: c.purpose,
        open: c.open,
        created_by: c.created_by,
        created_at: c.created_at,
        owner,
        member_count: c.member_count,
        members_preview: Vec::new(),
        is_member: false,
        new_messages: None,
    }
}

fn agent(a: records::Agent) -> AgentInfo {
    AgentInfo {
        lane: a.lane.map(|l| kkt_api_agent::AgentLane {
            name: l.name,
            version: l.version,
            current_version: l.current_version,
            taken_at: a.created_at.clone(),
        }),
        handle: a.handle,
        created_at: a.created_at,
        last_seen: a.last_seen,
        last_sent: None,
        run_id: a.run_id,
        description: a.description,
        client: None,
        listening: a.listening,
        waiting_channels: a.waiting_channels,
    }
}

fn link(c: records::ChannelLink) -> ChannelRef {
    ChannelRef {
        id: c.id,
        name: c.name,
    }
}

fn message(m: records::Message) -> Message {
    Message {
        id: m.id,
        channel_id: m.channel_id,
        channel_name: m.channel_name,
        agent: m.agent,
        body: m.body,
        created_at: m.created_at,
        run_id: m.run_id,
    }
}

fn run(r: records::ServerRun) -> ServerRunInfo {
    ServerRunInfo {
        id: r.id,
        started_at: r.started_at,
        version: r.version,
        git_commit: r.git_commit,
        settings: r.settings,
        instructions_sha256: r.instructions_sha256,
    }
}

fn activity(c: records::ChannelSummary) -> ChannelActivity {
    ChannelActivity {
        info: channel(c.channel, None),
        message_count: c.message_count,
        last_message_at: c.last_message_at,
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

fn lane_version(v: records::LaneVersion) -> LaneVersion {
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

fn lane_retired(r: records::LaneRetired) -> LaneRetired {
    LaneRetired {
        at: r.at,
        run_id: super::RUN_ID,
        by_admin: r.by_admin,
    }
}

fn lane(
    name: String,
    created_at: String,
    retired: Option<records::LaneRetired>,
    current: records::LaneVersion,
) -> Lane {
    Lane {
        name,
        created_at,
        run_id: super::RUN_ID,
        retired: retired.map(lane_retired),
        current: lane_version(current),
    }
}

fn kept_reason(r: records::KeptReason) -> KeptReason {
    match r {
        records::KeptReason::Gone => KeptReason::Gone,
        records::KeptReason::Active => KeptReason::Active,
        records::KeptReason::Owner => KeptReason::Owner,
    }
}

impl Api for Preview {
    fn in_swarm(&self, swarm: SwarmId) -> Arc<dyn Api> {
        Arc::new(Preview::in_swarm(self, swarm))
    }

    fn limits(&self) -> Limits {
        LIMITS
    }

    fn current_server_announcement(&self) -> Answer<'_, Option<ServerAnnouncement>> {
        refused()
    }
}

impl OperatorApi for Preview {
    fn in_swarm_operator(&self, swarm: SwarmId) -> Arc<dyn OperatorApi> {
        Arc::new(Preview::in_swarm(self, swarm))
    }
}

impl Views for Preview {
    fn overview<'a>(
        &'a self,
        _operator: &'a Operator,
        window: Duration,
        bucket: Duration,
    ) -> Answer<'a, Overview> {
        answer(Preview::overview(self, window, bucket), |o| Overview {
            channels: o.channels.into_iter().map(activity).collect(),
            agents: o
                .agents
                .into_iter()
                .map(|a| AgentActivity {
                    info: agent(a.agent),
                    channels: a.channels.into_iter().map(link).collect(),
                })
                .collect(),
            current_run: o.current_run.map(run),
            now: o.now,
        })
    }

    fn hidden_channels<'a>(
        &'a self,
        _operator: &'a Operator,
        window: Duration,
        bucket: Duration,
    ) -> Answer<'a, Vec<ChannelActivity>> {
        answer(Preview::hidden_channels(self, window, bucket), |hidden| {
            hidden.into_iter().map(activity).collect()
        })
    }

    fn channel_view<'a>(
        &'a self,
        _operator: &'a Operator,
        id: String,
        before_id: Option<i64>,
        page_messages: usize,
    ) -> Answer<'a, ChannelView> {
        answer(Preview::channel(self, id, before_id, page_messages), |v| {
            ChannelView {
                channel: channel(v.channel, v.owner),
                hidden: v.hidden,
                members: v.members.into_iter().map(agent).collect(),
                latest_messages: v.latest_messages.into_iter().map(message).collect(),
                messages: v.messages.into_iter().map(message).collect(),
                older_before_id: v.older_before_id,
            }
        })
    }

    fn agent_view<'a>(&'a self, _operator: &'a Operator, handle: String) -> Answer<'a, AgentView> {
        answer(Preview::agent(self, handle), |v| AgentView {
            memory: v
                .memory
                .into_iter()
                .map(|e| MemoryEntry {
                    handle: v.agent.handle.clone(),
                    name: e.name,
                    text: e.text,
                    saved_at: e.saved_at,
                })
                .collect(),
            agent: agent(v.agent),
            channels: v.channels.into_iter().map(|c| channel(c, None)).collect(),
            message_count: v.message_count,
        })
    }

    fn subscribe<'a>(&'a self, _operator: &'a Operator) -> Answer<'a, Box<dyn Notices>> {
        let notices: Box<dyn Notices> = Box::new(PreviewNotices(self.events.subscribe()));
        Box::pin(std::future::ready(Ok(notices)))
    }
}

/// The preview's events, as the operator's notices.
struct PreviewNotices(Subscription);

impl Notices for PreviewNotices {
    fn next(&mut self) -> NextNotice<'_> {
        Box::pin(async move { self.0.next().await.map(notice) })
    }
}

/// The events part's notice, as the operator hears it.
fn notice(heard: kkt_events::Notice) -> Notice {
    use kkt_events::Notice as Heard;
    match heard {
        Heard::Message(m) => Notice::Message(Message {
            id: m.id,
            channel_id: m.channel_id,
            channel_name: m.channel_name,
            agent: m.agent,
            body: m.body,
            created_at: m.created_at,
            run_id: m.run_id,
        }),
        Heard::Agent(a) => Notice::Agent(kkt_api_operator::NewAgent {
            handle: a.handle,
            created_at: a.created_at,
        }),
        Heard::Channel(c) => Notice::Channel(kkt_api_operator::ChannelCreated {
            id: c.id,
            name: c.name,
            open: c.open,
            created_by: c.created_by,
            member_count: c.member_count,
        }),
        Heard::Joined(j) => Notice::Joined(kkt_api_operator::Joined {
            agent: j.agent,
            channel_id: j.channel_id,
            channel_name: j.channel_name,
            member_count: j.member_count,
        }),
        Heard::Left(l) => Notice::Left(kkt_api_operator::Left {
            agent: l.agent,
            channel_id: l.channel_id,
            channel_name: l.channel_name,
            member_count: l.member_count,
            by: l.by,
            by_operator: l.by_operator,
        }),
        Heard::ChannelHidden(h) => Notice::ChannelHidden(kkt_api_operator::ChannelHidden {
            channel_id: h.channel_id,
            channel_name: h.channel_name,
            by: h.by,
            by_operator: h.by_operator,
        }),
        Heard::AgentHidden(h) => Notice::AgentHidden(kkt_api_operator::AgentHidden {
            agent: h.agent,
            by: h.by,
            by_operator: h.by_operator,
            channels: h
                .channels
                .into_iter()
                .map(|c| kkt_api_operator::ChannelMembers {
                    channel_id: c.channel_id,
                    channel_name: c.channel_name,
                    member_count: c.member_count,
                })
                .collect(),
        }),
        Heard::Missed => Notice::Missed,
    }
}

impl Changes for Preview {
    fn create_channel_as_operator(
        &self,
        _actor: Actor,
        new: NewChannel,
    ) -> Answer<'_, ChannelInfo> {
        let created = Preview::create_channel(
            self,
            new.name,
            new.purpose.unwrap_or_default(),
            new.open,
            new.members,
        );
        Box::pin(async move {
            let id = created.await.map_err(rule_error)?;
            self.channel_now(id).await
        })
    }

    fn change_channel(
        &self,
        _actor: Actor,
        channel_id: String,
        change: ChannelChange,
    ) -> Answer<'_, ()> {
        let action = match change {
            ChannelChange::SetOpen(open) => records::ChannelAction::SetOpen(open),
            ChannelChange::AddMember(handle) => records::ChannelAction::AddMember(handle),
            ChannelChange::RemoveMember(handle) => records::ChannelAction::RemoveMember(handle),
            ChannelChange::SetOwner(handle) => records::ChannelAction::SetOwner(handle),
            ChannelChange::Hide => records::ChannelAction::Hide,
        };
        answer(Preview::change_channel(self, channel_id, action), |()| ())
    }

    fn hide_agent(&self, _actor: Actor, handle: String) -> Answer<'_, ()> {
        answer(Preview::hide_agent(self, handle), |()| ())
    }

    fn stale_agents<'a>(
        &'a self,
        _operator: &'a Operator,
        idle: Duration,
    ) -> Answer<'a, Vec<StaleAgent>> {
        answer(Preview::stale_agents(self, idle), |stale| {
            stale
                .into_iter()
                .map(|s| StaleAgent {
                    info: agent(s.agent),
                    channels: s.channels.into_iter().map(link).collect(),
                    memory_entries: s.memory_entries,
                    owns: s.owns.into_iter().map(link).collect(),
                })
                .collect()
        })
    }

    fn hide_stale_agents(
        &self,
        _actor: Actor,
        idle: Duration,
        chosen: Vec<String>,
    ) -> Answer<'_, StaleHidden> {
        answer(Preview::hide_stale_agents(self, idle, chosen), |o| {
            StaleHidden {
                hidden: o.hidden,
                kept: o
                    .kept
                    .into_iter()
                    .map(|k| Kept {
                        handle: k.handle,
                        reason: kept_reason(k.reason),
                    })
                    .collect(),
            }
        })
    }

    fn new_recovery_code<'a>(
        &'a self,
        _operator: &'a Operator,
        handle: String,
    ) -> Answer<'a, String> {
        answer(Preview::new_recovery_code(self, handle), |code| code)
    }
}

impl kkt_api_operator::Lanes for Preview {
    fn lanes<'a>(&'a self, _operator: &'a Operator) -> Answer<'a, Vec<Lane>> {
        answer(Preview::lanes(self), |view| {
            view.lanes
                .into_iter()
                .map(|l| lane(l.name, l.created_at, l.retired, l.current))
                .collect()
        })
    }

    fn lanes_page<'a>(
        &'a self,
        _operator: &'a Operator,
        _limit: usize,
        _cursor: Option<String>,
    ) -> Answer<'a, Page<Lane>> {
        refused()
    }

    fn lane<'a>(&'a self, _operator: &'a Operator, _name: String) -> Answer<'a, Lane> {
        refused()
    }

    fn lane_history<'a>(
        &'a self,
        _operator: &'a Operator,
        name: String,
    ) -> Answer<'a, LaneHistory> {
        let found = Preview::lane(self, name.clone());
        Box::pin(async move {
            let versions = found.await.map_err(rule_error)?.history;
            Ok(LaneHistory {
                lane: self.lane_now(name).await?,
                versions: versions.into_iter().map(lane_version).collect(),
            })
        })
    }

    fn save_lane(&self, _actor: Actor, _save: LaneSave) -> Answer<'_, Lane> {
        refused()
    }

    fn create_lane(&self, _actor: Actor, save: LaneSave) -> Answer<'_, Lane> {
        self.saved_lane(save, true)
    }

    fn update_lane(&self, _actor: Actor, save: LaneSave) -> Answer<'_, Lane> {
        self.saved_lane(save, false)
    }

    fn retire_lane(&self, _actor: Actor, name: String) -> Answer<'_, Lane> {
        let retired = Preview::retire_lane(self, name.clone());
        Box::pin(async move {
            retired.await.map_err(rule_error)?;
            self.lane_now(name).await
        })
    }
}

impl Preview {
    /// Saves a lane as the preview's world does (`create`: a new one), and
    /// gives it back as it now is.
    fn saved_lane(&self, save: LaneSave, create: bool) -> Answer<'_, Lane> {
        let name = save.name.clone();
        let saved = Preview::save_lane(
            self,
            records::LaneSave {
                name: save.name,
                description: save.description,
                text: save.text,
            },
            create,
        );
        Box::pin(async move {
            saved.await.map_err(rule_error)?;
            self.lane_now(name).await
        })
    }

    /// The channel with this ID as it now is.
    async fn channel_now(&self, id: String) -> kkt_api_agent::Result<ChannelInfo> {
        let view = Preview::channel(self, id, None, 1)
            .await
            .map_err(rule_error)?;
        Ok(channel(view.channel, view.owner))
    }

    /// The lane named `name` as it now is.
    async fn lane_now(&self, name: String) -> kkt_api_agent::Result<Lane> {
        let view = Preview::lane(self, name).await.map_err(rule_error)?;
        let current = view
            .history
            .into_iter()
            .next()
            .ok_or_else(|| RuleError::new(Code::Internal, "the lane has no versions"))?;
        Ok(lane(view.name, view.created_at, view.retired, current))
    }
}

impl Server for Preview {
    fn server_announcement_history<'a>(
        &'a self,
        _operator: &'a Operator,
    ) -> Answer<'a, Vec<ServerAnnouncement>> {
        answer(Preview::server_announcement(self), |view| {
            view.history
                .into_iter()
                .map(|v| ServerAnnouncement {
                    version: v.version,
                    text: v.text,
                    saved_at: v.saved_at,
                    run_id: v.run_id,
                    saved_by_admin: v.saved_by_admin,
                })
                .collect()
        })
    }

    fn save_server_announcement(
        &self,
        _actor: Actor,
        text: String,
    ) -> Answer<'_, ServerAnnouncement> {
        let saved = Preview::save_server_announcement(self, text);
        Box::pin(async move {
            saved.await.map_err(rule_error)?;
            let history = Preview::server_announcement(self)
                .await
                .map_err(rule_error)?
                .history;
            let v = history
                .into_iter()
                .next()
                .ok_or_else(|| RuleError::new(Code::Internal, "the announcement was not saved"))?;
            Ok(ServerAnnouncement {
                version: v.version,
                text: v.text,
                saved_at: v.saved_at,
                run_id: v.run_id,
                saved_by_admin: v.saved_by_admin,
            })
        })
    }

    fn server_runs<'a>(&'a self, _operator: &'a Operator) -> Answer<'a, Vec<ServerRunInfo>> {
        answer(Preview::server_runs(self), |runs| {
            runs.into_iter().map(run).collect()
        })
    }
}

impl Channels for Preview {
    fn create_channel<'a>(&'a self, _me: &'a Agent, _new: NewChannel) -> Answer<'a, ChannelInfo> {
        refused()
    }

    fn list_channels<'a>(
        &'a self,
        _me: &'a Agent,
        _q: ChannelQuery,
    ) -> Answer<'a, Page<ChannelInfo>> {
        refused()
    }

    fn join_channel<'a>(&'a self, _me: &'a Agent, _channel_id: String) -> Answer<'a, ChannelInfo> {
        refused()
    }

    fn leave_channel<'a>(&'a self, _me: &'a Agent, _channel_id: String) -> Answer<'a, ()> {
        refused()
    }

    fn add_to_channel<'a>(
        &'a self,
        _me: &'a Agent,
        _channel_id: String,
        _agents: Vec<String>,
    ) -> Answer<'a, Members> {
        refused()
    }

    fn remove_from_channel<'a>(
        &'a self,
        _me: &'a Agent,
        _channel_id: String,
        _agent: String,
    ) -> Answer<'a, Members> {
        refused()
    }

    fn transfer_owner<'a>(
        &'a self,
        _me: &'a Agent,
        _channel_id: String,
        _agent: String,
    ) -> Answer<'a, ChannelInfo> {
        refused()
    }

    /// As the operator, the only actor the page has. The page uses only
    /// whether it worked.
    fn set_purpose(
        &self,
        _actor: Actor,
        channel_id: String,
        purpose: String,
    ) -> Answer<'_, ChannelInfo> {
        let said = Preview::set_purpose(self, channel_id.clone(), purpose);
        Box::pin(async move {
            said.await.map_err(rule_error)?;
            self.channel_now(channel_id).await
        })
    }

    fn set_note<'a>(
        &'a self,
        _me: &'a Agent,
        _channel_id: String,
        _text: String,
    ) -> Answer<'a, ()> {
        refused()
    }

    fn get_note<'a>(&'a self, _me: &'a Agent, _channel_id: String) -> Answer<'a, ChannelNote> {
        refused()
    }

    fn hide_channel<'a>(&'a self, _me: &'a Agent, _channel_id: String) -> Answer<'a, ()> {
        refused()
    }

    fn list_members<'a>(
        &'a self,
        _me: &'a Agent,
        _channel_id: String,
        _limit: usize,
        _cursor: Option<String>,
    ) -> Answer<'a, Members> {
        refused()
    }
}

impl Agents for Preview {
    fn authenticate<'a>(&'a self, _token: Option<&'a str>) -> Answer<'a, Agent> {
        refused()
    }

    fn register_with_client(
        &self,
        _client: Option<ClientInfo>,
        _proposed: Option<String>,
        _lane: Option<String>,
    ) -> Answer<'_, Registration> {
        refused()
    }

    fn reclaim_with_client(
        &self,
        _handle: String,
        _recovery_code: String,
        _client: Option<ClientInfo>,
        _lane: Option<String>,
    ) -> Answer<'_, Reclaimed> {
        refused()
    }

    fn unregister<'a>(&'a self, _me: &'a Agent) -> Answer<'a, ()> {
        refused()
    }

    fn set_description<'a>(&'a self, _me: &'a Agent, _text: String) -> Answer<'a, AgentInfo> {
        refused()
    }

    fn list_agents<'a>(
        &'a self,
        _me: &'a Agent,
        _query: Option<String>,
        _lane: Option<String>,
        _limit: usize,
        _cursor: Option<String>,
    ) -> Answer<'a, Page<AgentInfo>> {
        refused()
    }
}

impl Messages for Preview {
    fn send<'a>(
        &'a self,
        _me: &'a Agent,
        _channel_id: String,
        _body: String,
    ) -> Answer<'a, Message> {
        refused()
    }

    fn read<'a>(&'a self, _me: &'a Agent, _q: ReadQuery) -> Answer<'a, MessagePage> {
        refused()
    }

    fn get_message<'a>(&'a self, _me: &'a Agent, _id: i64) -> Answer<'a, Message> {
        refused()
    }

    fn search<'a>(&'a self, _me: &'a Agent, _q: SearchQuery) -> Answer<'a, Vec<SearchHit>> {
        refused()
    }

    fn activity<'a>(
        &'a self,
        _me: &'a Agent,
        _after_id: i64,
    ) -> Answer<'a, Vec<kkt_api_agent::ActivityRow>> {
        refused()
    }

    fn wait_for_messages<'a>(&'a self, _me: &'a Agent, _q: WaitQuery) -> Answer<'a, WaitResult> {
        refused()
    }
}

impl Memory for Preview {
    fn remember<'a>(
        &'a self,
        _me: &'a Agent,
        _name: String,
        _text: String,
    ) -> Answer<'a, MemorySaved> {
        refused()
    }

    fn recall<'a>(&'a self, _me: &'a Agent, _name: String) -> Answer<'a, MemoryEntry> {
        refused()
    }

    fn forget<'a>(&'a self, _me: &'a Agent, _name: String) -> Answer<'a, ()> {
        refused()
    }

    fn list_memories<'a>(
        &'a self,
        _me: &'a Agent,
        _limit: usize,
        _cursor: Option<String>,
    ) -> Answer<'a, MemoryList> {
        refused()
    }
}

impl kkt_api_agent::Lanes for Preview {
    fn held_lane<'a>(&'a self, _me: &'a Agent) -> Answer<'a, Option<HeldLane>> {
        refused()
    }

    fn take_lane<'a>(&'a self, _me: &'a Agent, _lane: Option<String>) -> Answer<'a, LaneTaking> {
        refused()
    }

    fn takeable_lanes_page(
        &self,
        _limit: usize,
        _cursor: Option<String>,
    ) -> Answer<'_, Page<Lane>> {
        refused()
    }
}
