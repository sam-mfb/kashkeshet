//! Turning the events part's notices into the live feed's events, for both
//! sources.

use super::source::{
    AgentHidden, ChannelHidden, ChannelMembers, Joined, Left, LiveEvent, Message, NewAgent,
    NewChannel,
};
use crate::events::Notice;
use crate::names;

pub fn live_event(notice: Notice) -> LiveEvent {
    match notice {
        Notice::Message(m) => LiveEvent::Message(Message {
            id: m.id,
            channel_id: m.channel_id,
            channel_name: m.channel_name,
            from_operator: m.agent == names::OPERATOR,
            agent: m.agent,
            body: m.body,
            created_at: m.created_at,
            run_id: m.run_id,
        }),
        Notice::Agent(a) => LiveEvent::Agent(NewAgent {
            name: a.name,
            created_at: a.created_at,
        }),
        Notice::Channel(c) => LiveEvent::Channel(NewChannel {
            id: c.id,
            name: c.name,
            open: c.open,
            created_by: c.created_by,
            member_count: c.member_count,
        }),
        Notice::Joined(j) => LiveEvent::Joined(Joined {
            agent: j.agent,
            channel_id: j.channel_id,
            channel_name: j.channel_name,
            member_count: j.member_count,
        }),
        Notice::Left(l) => LiveEvent::Left(Left {
            agent: l.agent,
            channel_id: l.channel_id,
            channel_name: l.channel_name,
            member_count: l.member_count,
            by: l.by,
            by_operator: l.by_operator,
        }),
        Notice::ChannelHidden(h) => LiveEvent::ChannelHidden(ChannelHidden {
            channel_id: h.channel_id,
            channel_name: h.channel_name,
            by: h.by,
            by_operator: h.by_operator,
        }),
        Notice::AgentHidden(h) => LiveEvent::AgentHidden(AgentHidden {
            agent: h.agent,
            by: h.by,
            by_operator: h.by_operator,
            channels: h
                .channels
                .into_iter()
                .map(|c| ChannelMembers {
                    channel_id: c.channel_id,
                    channel_name: c.channel_name,
                    member_count: c.member_count,
                })
                .collect(),
        }),
        Notice::Missed(n) => LiveEvent::Missed(n),
    }
}
