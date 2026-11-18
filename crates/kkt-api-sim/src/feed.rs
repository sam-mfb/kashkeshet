//! The preview's live feed: what happens in each swarm's world, as the
//! operator's notices, heard only by that swarm's subscribers. A subscriber
//! that falls behind hears [`Notice::Missed`], and every subscription ends
//! when the preview's shutdown signal stops.

use kkt_api_operator::{NextNotice, Notice, Notices};
use kkt_shutdown::Shutdown;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::broadcast::{self, error::RecvError};

/// Notices kept for each swarm for a slow subscriber before it starts
/// missing them, as many as kkt's own server keeps.
const BUFFER: usize = 1024;

/// Every swarm's feed, ending when `shutdown` stops.
#[derive(Clone)]
pub(crate) struct Feed {
    /// Each swarm's sender, made the first time the swarm is subscribed to.
    swarms: Arc<Mutex<HashMap<String, broadcast::Sender<Notice>>>>,
    shutdown: Shutdown,
}

impl Feed {
    pub(crate) fn new(shutdown: Shutdown) -> Feed {
        Feed {
            swarms: Arc::new(Mutex::new(HashMap::new())),
            shutdown,
        }
    }

    /// The feed as the swarm with this ID sees it.
    pub(crate) fn in_swarm(&self, swarm: &str) -> SwarmFeed {
        SwarmFeed {
            feed: self.clone(),
            swarm: swarm.to_string(),
        }
    }

    /// The map of swarms. Nothing panics while holding the lock, so a
    /// poisoned lock still holds a consistent map.
    fn swarms(&self) -> MutexGuard<'_, HashMap<String, broadcast::Sender<Notice>>> {
        self.swarms.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// One swarm's feed: [`Feed::in_swarm`].
#[derive(Clone)]
pub(crate) struct SwarmFeed {
    feed: Feed,
    swarm: String,
}

impl SwarmFeed {
    /// Tells this swarm's current subscribers about `notice`.
    pub(crate) fn publish(&self, notice: Notice) {
        let swarms = self.feed.swarms();
        // A swarm nobody has subscribed to has no sender: the world already
        // holds what happened.
        let Some(sender) = swarms.get(&self.swarm) else {
            return;
        };
        match sender.send(notice) {
            Ok(_receivers) => {}
            // Nobody listening now is normal.
            Err(_unheard) => {}
        }
    }

    /// Hears everything published in this swarm from now on.
    pub(crate) fn subscribe(&self) -> FeedNotices {
        let receiver = self
            .feed
            .swarms()
            .entry(self.swarm.clone())
            .or_insert_with(|| broadcast::channel(BUFFER).0)
            .subscribe();
        FeedNotices {
            receiver,
            shutdown: self.feed.shutdown.clone(),
        }
    }
}

/// A subscription to one swarm's feed.
pub(crate) struct FeedNotices {
    receiver: broadcast::Receiver<Notice>,
    shutdown: Shutdown,
}

impl Notices for FeedNotices {
    fn next(&mut self) -> NextNotice<'_> {
        Box::pin(async move {
            tokio::select! {
                biased;
                () = self.shutdown.stopped() => None,
                received = self.receiver.recv() => match received {
                    Ok(notice) => Some(notice),
                    Err(RecvError::Lagged(_)) => Some(Notice::Missed),
                    Err(RecvError::Closed) => None,
                },
            }
        })
    }
}
