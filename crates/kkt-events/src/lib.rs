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
//! Each swarm has a buffer of its own, so a busy swarm cannot make another
//! swarm's subscribers fall behind, and when a subscriber falls behind says
//! nothing about other swarms. A swarm's buffer exists only while the swarm
//! has a subscriber: publishing in a swarm nobody is subscribed to keeps
//! nothing, and the buffer goes when the last subscription does.
//!
//! Tokio's broadcast channel does the work, but stays inside this module.

use futures::Stream;
use kkt_shutdown::Shutdown;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
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
    bus: Arc<Bus>,
    shutdown: Option<Shutdown>,
}

/// The swarms that have subscribers, each with its own buffer. A swarm is
/// here exactly while it has a subscription: the first `subscribe` adds it
/// and the last subscription's drop removes it. Both count subscriptions in
/// `Swarm::subscriptions` under the lock, so a subscription that arrives as
/// the last one leaves is never left on a channel nobody publishes to, and
/// two last subscriptions leaving at once do not leave a buffer behind.
struct Bus {
    buffer: usize,
    swarms: Mutex<HashMap<Arc<str>, Swarm>>,
}

/// A swarm with subscribers: its buffer, and how many subscriptions it
/// has. The count is ours, not the channel's receiver count, which changes
/// only after a subscription's drop has let go of the lock.
struct Swarm {
    sender: broadcast::Sender<Published>,
    subscriptions: usize,
}

impl Bus {
    /// The map of swarms. Nothing panics while holding the lock, so a
    /// poisoned lock still holds a consistent map.
    fn swarms(&self) -> MutexGuard<'_, HashMap<Arc<str>, Swarm>> {
        self.swarms.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The bus's own settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Notices kept for each swarm for a slow subscriber before it starts
    /// missing them.
    pub buffer: usize,
}

impl Settings {
    /// 1024 notices per swarm.
    pub const DEFAULT: Settings = Settings { buffer: 1024 };
}

impl Events {
    /// A bus with `settings`.
    pub fn new(settings: Settings) -> Events {
        Events {
            bus: Arc::new(Bus {
                buffer: settings.buffer,
                swarms: Mutex::new(HashMap::new()),
            }),
            shutdown: None,
        }
    }

    /// How many swarms have a buffer now.
    #[cfg(test)]
    fn buffers(&self) -> usize {
        self.bus.swarms().len()
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
        let swarms = self.events.bus.swarms();
        // A swarm nobody is subscribed to has no buffer, and gets none:
        // what happened is already stored.
        let Some(swarm) = swarms.get(&self.swarm) else {
            return;
        };
        match swarm.sender.send(published) {
            Ok(_receivers) => {}
            // Cannot happen while the swarm is in the map, which it leaves
            // with its last subscriber; and nobody listening is normal.
            Err(_unheard) => {}
        }
    }

    /// Hears about everything published in this swarm from now on, until
    /// the subscription is closed ([`Subscription::closer`]).
    pub fn subscribe(&self) -> Subscription {
        let (closed, _) = watch::channel(false);
        let receiver = {
            let mut swarms = self.events.bus.swarms();
            match swarms.get_mut(&self.swarm) {
                Some(swarm) => {
                    swarm.subscriptions += 1;
                    swarm.sender.subscribe()
                }
                None => {
                    let (sender, receiver) = broadcast::channel(self.events.bus.buffer);
                    let swarm = Swarm {
                        sender,
                        subscriptions: 1,
                    };
                    swarms.insert(self.swarm.clone(), swarm);
                    receiver
                }
            }
        };
        Subscription {
            receiver,
            bus: Arc::downgrade(&self.events.bus),
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
    /// about is in the database. It says no number: subscribers have never
    /// been given one, and the web page's `lagged` event carries none.
    Missed,
}

/// One subscriber's view of one swarm's notices.
pub struct Subscription {
    receiver: broadcast::Receiver<Published>,
    /// The bus, to remove the swarm's buffer when this is its last
    /// subscription. Weak, so that the subscription ends once every
    /// `Events` handle is gone.
    bus: Weak<Bus>,
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
            Ok(published) => published,
            Err(RecvError::Lagged(_)) => return Some(Notice::Missed),
            Err(RecvError::Closed) => return None,
        };
        Some(match published {
            Published::Message(message) => Notice::Message(message),
            Published::Agent(agent) => Notice::Agent(agent),
            Published::Channel(channel) => Notice::Channel(channel),
            Published::Joined(joined) => Notice::Joined(joined),
            Published::Left(left) => Notice::Left(left),
            Published::ChannelHidden(hidden) => Notice::ChannelHidden(hidden),
            Published::AgentHidden(hidden) => Notice::AgentHidden(hidden),
        })
    }

    /// The notices as a stream, which ends when `next` would return `None`.
    pub fn into_stream(self) -> impl Stream<Item = Notice> {
        futures::stream::unfold(self, |mut subscription| async move {
            let notice = subscription.next().await?;
            Some((notice, subscription))
        })
    }
}

impl Drop for Subscription {
    /// Counts this subscription out of its swarm, and removes the swarm's
    /// buffer if it was the last.
    fn drop(&mut self) {
        let Some(bus) = self.bus.upgrade() else {
            return;
        };
        let mut swarms = bus.swarms();
        let Some(swarm) = swarms.get_mut(&self.swarm) else {
            return;
        };
        swarm.subscriptions -= 1;
        if swarm.subscriptions == 0 {
            swarms.remove(&self.swarm);
        }
        drop(swarms);
        // This subscription's receiver goes only after this returns, with
        // the lock let go: a test widens that gap.
        #[cfg(test)]
        tests::pause_before_the_receiver_goes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn shutdown_ends_pending_and_new_subscriptions_with_publishers_alive() {
        let shutdown = Shutdown::new();
        let events = Events::new(Settings::DEFAULT).until_stopped(shutdown.clone());
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
        let events = Events::new(Settings::DEFAULT);
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
        let events = Events::new(Settings::DEFAULT);
        let swarm = events.in_swarm("sw_a");
        let mut sub = swarm.subscribe();
        swarm.publish(message(1));
        swarm.publish(message(2));
        assert_eq!(id(sub.next().await), Some(1));
        assert_eq!(id(sub.next().await), Some(2));
    }

    #[tokio::test]
    async fn a_slow_subscriber_is_told_it_missed_some() {
        let events = Events::new(Settings { buffer: 2 });
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
        let events = Events::new(Settings::DEFAULT);
        let swarm = events.in_swarm("sw_a");
        let mut sub = swarm.subscribe();
        swarm.publish(message(1));
        drop((events, swarm));
        assert_eq!(id(sub.next().await), Some(1));
        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    async fn the_stream_gives_the_same_notices_and_then_ends() {
        let events = Events::new(Settings::DEFAULT);
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
        let events = Events::new(Settings::DEFAULT);
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
    async fn a_flood_in_another_swarm_does_not_make_a_subscriber_fall_behind() {
        let events = Events::new(Settings { buffer: 2 });
        let (mine, theirs) = (events.in_swarm("sw_a"), events.in_swarm("sw_b"));
        let mut sub = mine.subscribe();
        let mut flooded = theirs.subscribe();
        mine.publish(message(1));
        for i in 2..=100 {
            theirs.publish(message(i));
        }
        mine.publish(message(101));
        assert_eq!(id(sub.next().await), Some(1));
        assert_eq!(id(sub.next().await), Some(101));
        // The flooded swarm's own slow subscriber does fall behind.
        assert!(matches!(flooded.next().await, Some(Notice::Missed)));
    }

    #[tokio::test]
    async fn publishing_in_a_swarm_nobody_watches_keeps_nothing() {
        let events = Events::new(Settings::DEFAULT);
        let swarm = events.in_swarm("sw_a");
        for i in 1..=5 {
            swarm.publish(message(i));
        }
        assert_eq!(events.buffers(), 0);
        // A later subscriber hears only what comes after it.
        let mut sub = swarm.subscribe();
        swarm.publish(message(6));
        assert_eq!(id(sub.next().await), Some(6));
    }

    #[tokio::test]
    async fn a_swarms_buffer_goes_with_its_last_subscriber() {
        let events = Events::new(Settings::DEFAULT);
        let (a, b) = (events.in_swarm("sw_a"), events.in_swarm("sw_b"));
        let first = a.subscribe();
        let second = a.subscribe();
        let other = b.subscribe();
        assert_eq!(events.buffers(), 2);
        drop(first);
        assert_eq!(events.buffers(), 2);
        drop(second);
        assert_eq!(events.buffers(), 1);
        drop(other);
        assert_eq!(events.buffers(), 0);
        // Closing does not drop the buffer; dropping the subscription does.
        let closed = a.subscribe();
        closed.closer().close();
        assert_eq!(events.buffers(), 1);
        drop(closed);
        assert_eq!(events.buffers(), 0);
    }

    /// A subscription made while the swarm's last other one is being
    /// dropped either joins that buffer before it goes or makes a new one;
    /// either way it hears what is published next. Another thread keeps
    /// subscribing and dropping, so drops race the subscriptions here.
    #[test]
    fn a_subscriber_arriving_as_the_last_leaves_hears_what_follows() {
        use futures::FutureExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        let events = Events::new(Settings::DEFAULT);
        let swarm = events.in_swarm("sw_a");
        let done = Arc::new(AtomicBool::new(false));
        let churn = {
            let (swarm, done) = (swarm.clone(), done.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    drop(swarm.subscribe());
                }
            })
        };
        for i in 0..5_000 {
            let mut arriving = swarm.subscribe();
            swarm.publish(message(i));
            // Published before `next` is asked, so it is ready at once.
            let heard = arriving.next().now_or_never().flatten();
            assert_eq!(id(heard), Some(i), "round {i}");
        }
        done.store(true, Ordering::Relaxed);
        churn.join().expect("the churning thread panicked");
        assert_eq!(events.buffers(), 0);
    }

    thread_local! {
        /// Whether a subscription dropped on this thread pauses after
        /// letting go of the lock, before its receiver goes.
        static PAUSE_BEFORE_THE_RECEIVER_GOES: std::cell::Cell<bool> =
            const { std::cell::Cell::new(false) };
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the pause only widens a gap between the test's own threads"
    )]
    pub(super) fn pause_before_the_receiver_goes() {
        if PAUSE_BEFORE_THE_RECEIVER_GOES.get() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The swarm's last two subscriptions, dropped at once on two threads,
    /// leave no buffer behind, even when each receiver goes well after its
    /// drop has let go of the lock.
    #[test]
    fn the_last_two_subscribers_leaving_at_once_leave_no_buffer() {
        let events = Events::new(Settings::DEFAULT);
        let swarm = events.in_swarm("sw_a");
        for round in 0..20 {
            let start = Arc::new(std::sync::Barrier::new(2));
            let leaving: Vec<_> = [swarm.subscribe(), swarm.subscribe()]
                .into_iter()
                .map(|subscription| {
                    let start = start.clone();
                    std::thread::spawn(move || {
                        PAUSE_BEFORE_THE_RECEIVER_GOES.set(true);
                        start.wait();
                        drop(subscription);
                    })
                })
                .collect();
            for thread in leaving {
                thread.join().expect("a dropping thread panicked");
            }
            assert_eq!(events.buffers(), 0, "round {round}");
        }
    }

    #[tokio::test]
    async fn closing_ends_a_subscription_waiting_or_not() {
        let events = Events::new(Settings::DEFAULT);
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
