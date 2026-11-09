//! Notifications of what happens: new messages, new agents, channels and
//! channel members, and agents and channels hidden.
//!
//! When a message is stored, the sender publishes it here, and the rules do
//! the same when an agent registers, unregisters or is hidden, a channel is
//! created or hidden, or an agent joins or leaves one. The web page and any agent waiting for messages subscribe; waiting
//! agents look only at messages. A subscriber that falls behind is told it
//! missed some, not how many, and can fetch them from the database.
//!
//! There is one bus for the server, and every notice is published in a
//! swarm: a subscriber hears only its own swarm's ([`Events::in_swarm`]).
//! A subscription can be closed ([`Closer`]), which ends it.
//!
//! Tokio's broadcast channel does the work, but stays inside this module.

use futures::Stream;
use kkt_config::EVENT_BUFFER;
use kkt_shutdown::Shutdown;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};

#[derive(Clone, Debug, Serialize)]
pub struct NewMessage {
    pub id: i64,
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
    pub body: String,
    pub created_at: String,
    /// The server run that stored the message.
    pub run_id: Option<i64>,
}

/// An agent registered.
#[derive(Clone, Debug, Serialize)]
pub struct NewAgent {
    pub handle: String,
    pub created_at: String,
}

/// A channel was created. Closed channels are announced too: only the
/// human's view hears these.
#[derive(Clone, Debug, Serialize)]
pub struct NewChannel {
    pub id: String,
    pub name: String,
    pub open: bool,
    pub created_by: String,
    pub created_at: String,
    /// The creator and anyone added at creation.
    pub member_count: i64,
}

/// An agent joined a channel it was not in, or was added to it.
#[derive(Clone, Debug, Serialize)]
pub struct Joined {
    pub agent: String,
    pub channel_id: String,
    pub channel_name: String,
    /// The channel's members afterwards.
    pub member_count: i64,
}

/// An agent left a channel, or was removed from it.
#[derive(Clone, Debug, Serialize)]
pub struct Left {
    pub agent: String,
    pub channel_id: String,
    pub channel_name: String,
    /// The channel's members afterwards.
    pub member_count: i64,
    /// Who made it leave: the agent itself, the channel's owner, or the
    /// operator.
    pub by: String,
    /// `by` is the human operator, who has no agent page.
    pub by_operator: bool,
}

/// A channel was hidden, by its owner or the operator. Its members were
/// removed; no `Left` is published for them.
#[derive(Clone, Debug, Serialize)]
pub struct ChannelHidden {
    pub channel_id: String,
    pub channel_name: String,
    /// The owner that hid it, or the operator.
    pub by: String,
    /// `by` is the human operator, who has no agent page.
    pub by_operator: bool,
}

/// An agent's avatar was removed: it unregistered itself, or the operator
/// hid it. It left every channel it was in; no `Left` is published for them.
#[derive(Clone, Debug, Serialize)]
pub struct AgentHidden {
    pub agent: String,
    /// The agent itself when it unregistered, or the operator.
    pub by: String,
    /// `by` is the human operator, who has no agent page.
    pub by_operator: bool,
    /// The channels it left, with their members afterwards.
    pub channels: Vec<ChannelMembers>,
}

/// A channel and its member count.
#[derive(Clone, Debug, Serialize)]
pub struct ChannelMembers {
    pub channel_id: String,
    pub channel_name: String,
    pub member_count: i64,
}

/// Everything that can be published.
#[derive(Clone, Debug)]
enum Published {
    Message(NewMessage),
    Agent(NewAgent),
    Channel(NewChannel),
    Joined(Joined),
    Left(Left),
    ChannelHidden(ChannelHidden),
    AgentHidden(AgentHidden),
}

/// The server's one bus. Every notice is published in a swarm and heard
/// only by subscribers to that swarm: [`Events::in_swarm`].
#[derive(Clone)]
pub struct Events {
    sender: broadcast::Sender<Sent>,
    shutdown: Option<Shutdown>,
}

/// A notice on the bus, with the swarm it was published in.
#[derive(Clone, Debug)]
struct Sent {
    swarm: Arc<str>,
    published: Published,
}

impl Events {
    pub fn new() -> Events {
        Events::with_buffer(EVENT_BUFFER)
    }

    /// Keeps `buffer` notices, of every swarm, for a slow subscriber before
    /// it starts missing them.
    pub fn with_buffer(buffer: usize) -> Events {
        let (sender, _) = broadcast::channel(buffer);
        Events {
            sender,
            shutdown: None,
        }
    }

    /// Ends subscriptions when the server stops, even while publishers exist.
    pub fn until_stopped(mut self, shutdown: Shutdown) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// The bus as one swarm sees it: what it publishes is heard only by its
    /// own subscribers, and they hear nothing from any other swarm. `swarm`
    /// is the swarm's ID.
    pub fn in_swarm(&self, swarm: &str) -> SwarmEvents {
        SwarmEvents {
            events: self.clone(),
            swarm: swarm.into(),
        }
    }
}

/// One swarm's notices: [`Events::in_swarm`].
#[derive(Clone)]
pub struct SwarmEvents {
    events: Events,
    swarm: Arc<str>,
}

impl SwarmEvents {
    /// Tells every current subscriber to this swarm about a new message.
    pub fn publish(&self, message: NewMessage) {
        self.send(Published::Message(message));
    }

    /// Tells every current subscriber that an agent registered.
    pub fn publish_agent(&self, agent: NewAgent) {
        self.send(Published::Agent(agent));
    }

    /// Tells every current subscriber that a channel was created.
    pub fn publish_channel(&self, channel: NewChannel) {
        self.send(Published::Channel(channel));
    }

    /// Tells every current subscriber that an agent joined a channel.
    pub fn publish_joined(&self, joined: Joined) {
        self.send(Published::Joined(joined));
    }

    /// Tells every current subscriber that an agent left a channel.
    pub fn publish_left(&self, left: Left) {
        self.send(Published::Left(left));
    }

    /// Tells every current subscriber that a channel was hidden.
    pub fn publish_channel_hidden(&self, hidden: ChannelHidden) {
        self.send(Published::ChannelHidden(hidden));
    }

    /// Tells every current subscriber that an agent was hidden or
    /// unregistered.
    pub fn publish_agent_hidden(&self, hidden: AgentHidden) {
        self.send(Published::AgentHidden(hidden));
    }

    fn send(&self, published: Published) {
        let sent = Sent {
            swarm: self.swarm.clone(),
            published,
        };
        match self.events.sender.send(sent) {
            Ok(_receivers) => {}
            // Nobody is listening. That is normal: what happened is already stored.
            Err(_unheard) => {}
        }
    }

    /// Hears about everything published in this swarm from now on, until
    /// the subscription is closed ([`Subscription::closer`]).
    pub fn subscribe(&self) -> Subscription {
        let (closed, _) = watch::channel(false);
        Subscription {
            receiver: self.events.sender.subscribe(),
            swarm: self.swarm.clone(),
            shutdown: self.events.shutdown.clone(),
            closed: Arc::new(closed),
        }
    }
}

/// What a subscriber hears.
#[derive(Clone, Debug)]
pub enum Notice {
    /// A new message.
    Message(NewMessage),
    /// A new agent.
    Agent(NewAgent),
    /// A new channel.
    Channel(NewChannel),
    /// An agent joined a channel.
    Joined(Joined),
    /// An agent left a channel.
    Left(Left),
    /// A channel was hidden.
    ChannelHidden(ChannelHidden),
    /// An agent was hidden or unregistered.
    AgentHidden(AgentHidden),
    /// The subscriber fell behind and missed some notices. What they were
    /// about is in the database. It says no number: the bus is shared by
    /// every swarm, so how many were missed would tell of other swarms.
    Missed,
}

/// One subscriber's view of one swarm's notices.
pub struct Subscription {
    receiver: broadcast::Receiver<Sent>,
    swarm: Arc<str>,
    shutdown: Option<Shutdown>,
    closed: Arc<watch::Sender<bool>>,
}

/// Closes a [`Subscription`]: it then ends, as if nothing more could be
/// published. Closing twice, or after the subscription is gone, does
/// nothing.
#[derive(Clone)]
pub struct Closer(Arc<watch::Sender<bool>>);

impl Closer {
    pub fn close(&self) {
        self.0.send_replace(true);
    }
}

impl Subscription {
    /// What closes this subscription, for whoever decides when the
    /// subscriber's right to hear ends.
    pub fn closer(&self) -> Closer {
        Closer(self.closed.clone())
    }

    /// The next notice in the subscription's swarm, or `None` once nothing
    /// more can be published (every `Events` handle is gone), the server's
    /// shutdown signal is stopped, or the subscription is closed.
    pub async fn next(&mut self) -> Option<Notice> {
        let mut closed = self.closed.subscribe();
        loop {
            let received = tokio::select! {
                biased;
                () = async {
                    match &self.shutdown {
                        Some(shutdown) => shutdown.stopped().await,
                        None => std::future::pending().await,
                    }
                } => return None,
                // The sender lives as long as this subscription, so waiting
                // fails only once it is gone, and then nothing is waiting.
                _ = closed.wait_for(|closed| *closed) => return None,
                received = self.receiver.recv() => received,
            };
            let published = match received {
                Ok(sent) if sent.swarm != self.swarm => continue,
                Ok(sent) => sent.published,
                Err(RecvError::Lagged(_)) => return Some(Notice::Missed),
                Err(RecvError::Closed) => return None,
            };
            return Some(match published {
                Published::Message(message) => Notice::Message(message),
                Published::Agent(agent) => Notice::Agent(agent),
                Published::Channel(channel) => Notice::Channel(channel),
                Published::Joined(joined) => Notice::Joined(joined),
                Published::Left(left) => Notice::Left(left),
                Published::ChannelHidden(hidden) => Notice::ChannelHidden(hidden),
                Published::AgentHidden(hidden) => Notice::AgentHidden(hidden),
            });
        }
    }

    /// The notices as a stream, which ends when `next` would return `None`.
    pub fn into_stream(self) -> impl Stream<Item = Notice> {
        futures::stream::unfold(self, |mut subscription| async move {
            let notice = subscription.next().await?;
            Some((notice, subscription))
        })
    }
}

impl Default for Events {
    fn default() -> Self {
        Events::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn shutdown_ends_pending_and_new_subscriptions_with_publishers_alive() {
        let shutdown = Shutdown::new();
        let events = Events::new().until_stopped(shutdown.clone());
        let swarm = events.in_swarm("sw_a");
        let mut sub = swarm.subscribe();
        let next = sub.next();
        tokio::pin!(next);
        assert!(futures::poll!(&mut next).is_pending());
        shutdown.stop();
        assert!(next.await.is_none());
        swarm.publish(message(1));
        assert!(swarm.subscribe().next().await.is_none());
    }

    fn message(id: i64) -> NewMessage {
        NewMessage {
            id,
            channel_id: "ch_test".into(),
            channel_name: "test".into(),
            agent: "a".into(),
            body: "hi".into(),
            created_at: "2026-10-23T14:00:00Z".into(),
            run_id: None,
        }
    }

    fn id(notice: Option<Notice>) -> Option<i64> {
        match notice {
            Some(Notice::Message(m)) => Some(m.id),
            other => panic!("expected a message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_subscriber_hears_every_kind_in_order() {
        let events = Events::new();
        let swarm = events.in_swarm("sw_a");
        let mut sub = swarm.subscribe();
        swarm.publish_agent(NewAgent {
            handle: "amber-falcon".into(),
            created_at: "2026-10-23T14:00:00Z".into(),
        });
        swarm.publish_channel(NewChannel {
            id: "ch_test".into(),
            name: "test".into(),
            open: false,
            created_by: "amber-falcon".into(),
            created_at: "2026-10-23T14:00:00Z".into(),
            member_count: 1,
        });
        swarm.publish_joined(Joined {
            agent: "bold-panther".into(),
            channel_id: "ch_test".into(),
            channel_name: "test".into(),
            member_count: 2,
        });
        swarm.publish_left(Left {
            agent: "bold-panther".into(),
            channel_id: "ch_test".into(),
            channel_name: "test".into(),
            member_count: 1,
            by: "bold-panther".into(),
            by_operator: false,
        });
        swarm.publish_channel_hidden(ChannelHidden {
            channel_id: "ch_test".into(),
            channel_name: "test".into(),
            by: "amber-falcon".into(),
            by_operator: false,
        });
        swarm.publish_agent_hidden(AgentHidden {
            agent: "bold-panther".into(),
            by: "operator".into(),
            by_operator: true,
            channels: Vec::new(),
        });
        swarm.publish(message(1));
        assert!(matches!(sub.next().await, Some(Notice::Agent(a)) if a.handle == "amber-falcon"));
        assert!(
            matches!(sub.next().await, Some(Notice::Channel(c)) if c.id == "ch_test" && !c.open)
        );
        assert!(
            matches!(sub.next().await, Some(Notice::Joined(j)) if j.agent == "bold-panther" && j.member_count == 2)
        );
        assert!(matches!(sub.next().await, Some(Notice::Left(l)) if l.member_count == 1));
        assert!(
            matches!(sub.next().await, Some(Notice::ChannelHidden(h)) if h.by == "amber-falcon")
        );
        assert!(matches!(sub.next().await, Some(Notice::AgentHidden(h)) if h.by_operator));
        assert_eq!(id(sub.next().await), Some(1));
    }

    #[tokio::test]
    async fn a_subscriber_hears_messages_in_order() {
        let events = Events::new();
        let swarm = events.in_swarm("sw_a");
        let mut sub = swarm.subscribe();
        swarm.publish(message(1));
        swarm.publish(message(2));
        assert_eq!(id(sub.next().await), Some(1));
        assert_eq!(id(sub.next().await), Some(2));
    }

    #[tokio::test]
    async fn a_slow_subscriber_is_told_it_missed_some() {
        let events = Events::with_buffer(2);
        let swarm = events.in_swarm("sw_a");
        let mut sub = swarm.subscribe();
        for i in 1..=5 {
            swarm.publish(message(i));
        }
        assert!(matches!(sub.next().await, Some(Notice::Missed)));
        assert_eq!(id(sub.next().await), Some(4));
    }

    #[tokio::test]
    async fn the_subscription_ends_when_events_are_gone() {
        let events = Events::new();
        let swarm = events.in_swarm("sw_a");
        let mut sub = swarm.subscribe();
        swarm.publish(message(1));
        drop((events, swarm));
        assert_eq!(id(sub.next().await), Some(1));
        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    async fn the_stream_gives_the_same_notices_and_then_ends() {
        let events = Events::new();
        let swarm = events.in_swarm("sw_a");
        let stream = swarm.subscribe().into_stream();
        swarm.publish(message(1));
        swarm.publish(message(2));
        drop((events, swarm));
        let ids: Vec<i64> = stream
            .map(|notice| match notice {
                Notice::Message(m) => m.id,
                other => panic!("expected a message, got {other:?}"),
            })
            .collect()
            .await;
        assert_eq!(ids, vec![1, 2]);
    }

    #[tokio::test]
    async fn a_subscriber_hears_only_its_own_swarm() {
        let events = Events::new();
        let (a, b) = (events.in_swarm("sw_a"), events.in_swarm("sw_b"));
        let mut hears_a = a.subscribe();
        let mut hears_b = b.subscribe();
        b.publish(message(1));
        a.publish(message(2));
        b.publish(message(3));
        drop((events, a, b));
        assert_eq!(id(hears_a.next().await), Some(2));
        assert!(hears_a.next().await.is_none());
        assert_eq!(id(hears_b.next().await), Some(1));
        assert_eq!(id(hears_b.next().await), Some(3));
        assert!(hears_b.next().await.is_none());
    }

    #[tokio::test]
    async fn a_lag_caused_by_another_swarm_says_no_number() {
        let events = Events::with_buffer(2);
        let (mine, theirs) = (events.in_swarm("sw_a"), events.in_swarm("sw_b"));
        let mut sub = mine.subscribe();
        mine.publish(message(1));
        for i in 2..=6 {
            theirs.publish(message(i));
        }
        mine.publish(message(7));
        assert!(matches!(sub.next().await, Some(Notice::Missed)));
        assert_eq!(id(sub.next().await), Some(7));
    }

    #[tokio::test]
    async fn closing_ends_a_subscription_waiting_or_not() {
        let events = Events::new();
        let swarm = events.in_swarm("sw_a");
        let mut sub = swarm.subscribe();
        let other = swarm.subscribe();
        let closer = sub.closer();
        {
            let next = sub.next();
            tokio::pin!(next);
            assert!(futures::poll!(&mut next).is_pending());
            closer.close();
            assert!(next.await.is_none());
        }
        // Closed for good: what is published later is not heard, and
        // closing again changes nothing.
        swarm.publish(message(1));
        closer.close();
        assert!(sub.next().await.is_none());
        // Other subscriptions go on.
        let mut other = other;
        assert_eq!(id(other.next().await), Some(1));
        // Closing after the subscription is gone does nothing.
        drop(sub);
        closer.close();
    }
}
