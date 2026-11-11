//! The rules as kkt-api-operator's contract: each operation is the rules'
//! own method of the same name, which checks the operator's permissions.

use super::*;
use kkt_api_agent::Answer;
use kkt_api_operator::{Changes, Lanes, Server, Views};
use std::time::Duration;

impl Views for Rules {
    fn overview<'a>(
        &'a self,
        operator: &'a Operator,
        window: Duration,
        bucket: Duration,
    ) -> Answer<'a, Overview> {
        Box::pin(Rules::overview(self, operator, window, bucket))
    }

    fn hidden_channels<'a>(
        &'a self,
        operator: &'a Operator,
        window: Duration,
        bucket: Duration,
    ) -> Answer<'a, Vec<ChannelActivity>> {
        Box::pin(Rules::hidden_channels(self, operator, window, bucket))
    }

    fn channel_view<'a>(
        &'a self,
        operator: &'a Operator,
        id: String,
        before_id: Option<i64>,
        page_messages: usize,
    ) -> Answer<'a, ChannelView> {
        Box::pin(Rules::channel_view(
            self,
            operator,
            id,
            before_id,
            page_messages,
        ))
    }

    fn agent_view<'a>(&'a self, operator: &'a Operator, handle: String) -> Answer<'a, AgentView> {
        Box::pin(Rules::agent_view(self, operator, handle))
    }
}

impl Changes for Rules {
    fn create_channel_as_operator(&self, actor: Actor, new: NewChannel) -> Answer<'_, ChannelInfo> {
        Box::pin(Rules::create_channel_as_operator(self, actor, new))
    }

    fn change_channel(
        &self,
        actor: Actor,
        channel_id: String,
        change: ChannelChange,
    ) -> Answer<'_, ()> {
        Box::pin(Rules::change_channel(self, actor, channel_id, change))
    }

    fn hide_agent(&self, actor: Actor, handle: String) -> Answer<'_, ()> {
        Box::pin(Rules::hide_agent(self, actor, handle))
    }

    fn stale_agents<'a>(
        &'a self,
        operator: &'a Operator,
        idle: Duration,
    ) -> Answer<'a, Vec<StaleAgent>> {
        Box::pin(Rules::stale_agents(self, operator, idle))
    }

    fn hide_stale_agents(
        &self,
        actor: Actor,
        idle: Duration,
        chosen: Vec<String>,
    ) -> Answer<'_, StaleHidden> {
        Box::pin(Rules::hide_stale_agents(self, actor, idle, chosen))
    }

    fn new_recovery_code<'a>(
        &'a self,
        operator: &'a Operator,
        handle: String,
    ) -> Answer<'a, String> {
        Box::pin(Rules::new_recovery_code(self, operator, handle))
    }
}

impl Lanes for Rules {
    fn lanes<'a>(&'a self, operator: &'a Operator) -> Answer<'a, Vec<Lane>> {
        Box::pin(Rules::lanes(self, operator))
    }

    fn lanes_page<'a>(
        &'a self,
        operator: &'a Operator,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Page<Lane>> {
        Box::pin(Rules::lanes_page(self, operator, limit, cursor))
    }

    fn lane<'a>(&'a self, operator: &'a Operator, name: String) -> Answer<'a, Lane> {
        Box::pin(Rules::lane(self, operator, name))
    }

    fn lane_history<'a>(&'a self, operator: &'a Operator, name: String) -> Answer<'a, LaneHistory> {
        Box::pin(Rules::lane_history(self, operator, name))
    }

    fn save_lane(&self, actor: Actor, save: LaneSave) -> Answer<'_, Lane> {
        Box::pin(Rules::save_lane(self, actor, save))
    }

    fn create_lane(&self, actor: Actor, save: LaneSave) -> Answer<'_, Lane> {
        Box::pin(Rules::create_lane(self, actor, save))
    }

    fn update_lane(&self, actor: Actor, save: LaneSave) -> Answer<'_, Lane> {
        Box::pin(Rules::update_lane(self, actor, save))
    }

    fn retire_lane(&self, actor: Actor, name: String) -> Answer<'_, Lane> {
        Box::pin(Rules::retire_lane(self, actor, name))
    }
}

impl Server for Rules {
    fn server_announcement_history<'a>(
        &'a self,
        operator: &'a Operator,
    ) -> Answer<'a, Vec<ServerAnnouncement>> {
        Box::pin(Rules::server_announcement_history(self, operator))
    }

    fn save_server_announcement(
        &self,
        actor: Actor,
        text: String,
    ) -> Answer<'_, ServerAnnouncement> {
        Box::pin(Rules::save_server_announcement(self, actor, text))
    }

    fn server_runs<'a>(&'a self, operator: &'a Operator) -> Answer<'a, Vec<ServerRunInfo>> {
        Box::pin(Rules::server_runs(self, operator))
    }
}
