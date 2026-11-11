//! What the human who runs the server may do: the web page's operator
//! powers, by area ([`Views`], [`Changes`], [`Lanes`], [`Server`]), with the
//! records only they use. The records every level shares are kkt-api-agent's.
//!
//! A contract crate: traits and plain values. kkt-rules implements it; each
//! operation checks the operator's permissions itself. Every operation gives
//! back an [`Answer`], a boxed future, so the traits work through `dyn`.

mod changes;
mod lanes;
mod stale;
mod views;

pub use changes::*;
pub use lanes::*;
pub use stale::*;
pub use views::*;

use kkt_api_agent::{
    Actor, AgentInfo, AgentView, ChannelInfo, ChannelRef, Lane, LaneVersion, Message, NewChannel,
    Operator, Page, Result, ServerAnnouncement,
};
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// What an operation gives back: a future of its result.
pub type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What the operator sees: the overview, with each channel's activity
/// counted in buckets of `bucket` over the last `window`; hidden channels;
/// and one channel or agent in full.
pub trait Views: Send + Sync {
    fn overview<'a>(
        &'a self,
        operator: &'a Operator,
        window: Duration,
        bucket: Duration,
    ) -> Answer<'a, Overview>;

    fn hidden_channels<'a>(
        &'a self,
        operator: &'a Operator,
        window: Duration,
        bucket: Duration,
    ) -> Answer<'a, Vec<ChannelActivity>>;

    /// One channel and a page of `page_messages` of its messages, before
    /// `before_id` if given.
    fn channel_view<'a>(
        &'a self,
        operator: &'a Operator,
        id: String,
        before_id: Option<i64>,
        page_messages: usize,
    ) -> Answer<'a, ChannelView>;

    fn agent_view<'a>(&'a self, operator: &'a Operator, handle: String) -> Answer<'a, AgentView>;
}

/// What the operator changes: creating and changing channels as the
/// operator, hiding agents, finding and hiding stale agents, and new
/// recovery codes.
pub trait Changes: Send + Sync {
    fn create_channel_as_operator(&self, actor: Actor, new: NewChannel) -> Answer<'_, ChannelInfo>;

    fn change_channel(
        &self,
        actor: Actor,
        channel_id: String,
        change: ChannelChange,
    ) -> Answer<'_, ()>;

    fn hide_agent(&self, actor: Actor, handle: String) -> Answer<'_, ()>;

    /// Agents quiet for at least `idle`.
    fn stale_agents<'a>(
        &'a self,
        operator: &'a Operator,
        idle: Duration,
    ) -> Answer<'a, Vec<StaleAgent>>;

    /// Hides those of `chosen` that are still quiet for at least `idle`.
    fn hide_stale_agents(
        &self,
        actor: Actor,
        idle: Duration,
        chosen: Vec<String>,
    ) -> Answer<'_, StaleHidden>;

    /// A new recovery code for `handle`; the old one stops working.
    fn new_recovery_code<'a>(
        &'a self,
        operator: &'a Operator,
        handle: String,
    ) -> Answer<'a, String>;
}

/// Lanes as the operator manages them: listing (whole or by page), one lane
/// and its history, saving (create or update), creating and updating
/// separately, and retiring.
pub trait Lanes: Send + Sync {
    fn lanes<'a>(&'a self, operator: &'a Operator) -> Answer<'a, Vec<Lane>>;

    fn lanes_page<'a>(
        &'a self,
        operator: &'a Operator,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Page<Lane>>;

    fn lane<'a>(&'a self, operator: &'a Operator, name: String) -> Answer<'a, Lane>;

    fn lane_history<'a>(&'a self, operator: &'a Operator, name: String) -> Answer<'a, LaneHistory>;

    /// Creates the lane, or saves its next version.
    fn save_lane(&self, actor: Actor, save: LaneSave) -> Answer<'_, Lane>;

    /// As [`Lanes::save_lane`], but only a new lane.
    fn create_lane(&self, actor: Actor, save: LaneSave) -> Answer<'_, Lane>;

    /// As [`Lanes::save_lane`], but only an existing lane.
    fn update_lane(&self, actor: Actor, save: LaneSave) -> Answer<'_, Lane>;

    fn retire_lane(&self, actor: Actor, name: String) -> Answer<'_, Lane>;
}

/// The server's own records, as the operator manages them: the
/// announcement's history, saving it, and the server runs.
pub trait Server: Send + Sync {
    fn server_announcement_history<'a>(
        &'a self,
        operator: &'a Operator,
    ) -> Answer<'a, Vec<ServerAnnouncement>>;

    fn save_server_announcement(
        &self,
        actor: Actor,
        text: String,
    ) -> Answer<'_, ServerAnnouncement>;

    fn server_runs<'a>(&'a self, operator: &'a Operator) -> Answer<'a, Vec<ServerRunInfo>>;
}
