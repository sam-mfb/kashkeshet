//! Caps on how fast things can happen: messages per agent (an agent is a
//! handle in a swarm), and registrations across the whole server.
//!
//! Each cap is a bucket that holds up to `per_minute` uses and refills at
//! `per_minute` uses per minute. State lives in memory and resets on restart.

use crate::SwarmId;
use kkt_clock::Time;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Bucket {
    available: f64,
    updated: Time,
}

/// Takes one send from `bucket` at time `now`. Returns the new bucket and
/// whether the send is allowed.
fn take(bucket: Option<Bucket>, now: Time, per_minute: u32) -> (Bucket, bool) {
    let capacity = f64::from(per_minute);
    let available = match bucket {
        None => capacity,
        Some(b) => {
            let elapsed = (now - b.updated).num_milliseconds().max(0) as f64 / 60_000.0;
            (b.available + elapsed * capacity).min(capacity)
        }
    };
    if available >= 1.0 {
        (
            Bucket {
                available: available - 1.0,
                updated: now,
            },
            true,
        )
    } else {
        (
            Bucket {
                available,
                updated: now,
            },
            false,
        )
    }
}

/// Gives one send back to `bucket`, never filling it past `per_minute`.
fn refund(bucket: Bucket, per_minute: u32) -> Bucket {
    Bucket {
        available: (bucket.available + 1.0).min(f64::from(per_minute)),
        updated: bucket.updated,
    }
}

pub struct SendLimiter {
    per_minute: u32,
    buckets: Mutex<HashMap<(SwarmId, String), Bucket>>,
}

impl SendLimiter {
    pub fn new(per_minute: u32) -> SendLimiter {
        SendLimiter {
            per_minute,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Records one send by `agent` of `swarm` at `now`. Returns false if
    /// the agent is over its limit.
    pub fn allow(&self, swarm: &SwarmId, agent: &str, now: Time) -> bool {
        // A poisoned lock means another thread panicked mid-update; the map is
        // still usable because each entry is replaced whole.
        let mut buckets = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        let key = (swarm.clone(), agent.to_string());
        let (bucket, allowed) = take(buckets.get(&key).copied(), now, self.per_minute);
        buckets.insert(key, bucket);
        allowed
    }

    /// Gives back a send that `allow` counted but that did not happen.
    pub fn refund(&self, swarm: &SwarmId, agent: &str) {
        // See `allow` on why a poisoned lock is still usable.
        let mut buckets = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        let key = (swarm.clone(), agent.to_string());
        if let Some(bucket) = buckets.get(&key).copied() {
            buckets.insert(key, refund(bucket, self.per_minute));
        }
    }
}

/// One bucket for the whole server's registrations.
pub struct RegisterLimiter {
    per_minute: u32,
    bucket: Mutex<Option<Bucket>>,
}

impl RegisterLimiter {
    pub fn new(per_minute: u32) -> RegisterLimiter {
        RegisterLimiter {
            per_minute,
            bucket: Mutex::new(None),
        }
    }

    /// Records one registration at `now`. Returns false if the server is over
    /// its limit.
    pub fn allow(&self, now: Time) -> bool {
        // See `SendLimiter::allow` on why a poisoned lock is still usable.
        let mut bucket = self.bucket.lock().unwrap_or_else(|p| p.into_inner());
        let (next, allowed) = take(*bucket, now, self.per_minute);
        *bucket = Some(next);
        allowed
    }

    /// Gives back a registration that `allow` counted but that did not happen.
    pub fn refund(&self) {
        // See `SendLimiter::allow` on why a poisoned lock is still usable.
        let mut bucket = self.bucket.lock().unwrap_or_else(|p| p.into_inner());
        *bucket = bucket.map(|b| refund(b, self.per_minute));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_clock::Span;

    #[test]
    fn allows_a_burst_then_refills() {
        let main = SwarmId::main();
        let limiter = SendLimiter::new(3);
        let t0 = Time::from_parts(2026, 1, 1, 0, 0, 0).unwrap();
        assert!(limiter.allow(&main, "a", t0));
        assert!(limiter.allow(&main, "a", t0));
        assert!(limiter.allow(&main, "a", t0));
        assert!(!limiter.allow(&main, "a", t0));
        // Another agent has its own bucket, and so does the same handle in
        // another swarm.
        assert!(limiter.allow(&main, "b", t0));
        assert!(limiter.allow(&SwarmId::new("sw_other"), "a", t0));
        // One third of a minute refills one send.
        assert!(limiter.allow(&main, "a", t0 + Span::seconds(20)));
        assert!(!limiter.allow(&main, "a", t0 + Span::seconds(20)));
    }

    #[test]
    fn refund_gives_back_one_send_up_to_capacity() {
        let t0 = Time::from_parts(2026, 1, 1, 0, 0, 0).unwrap();
        let (empty, _) = take(
            Some(Bucket {
                available: 1.0,
                updated: t0,
            }),
            t0,
            3,
        );
        assert_eq!(
            refund(empty, 3),
            Bucket {
                available: 1.0,
                updated: t0
            }
        );
        let full = Bucket {
            available: 3.0,
            updated: t0,
        };
        assert_eq!(refund(full, 3), full);
    }

    #[test]
    fn a_refunded_send_can_be_used_again() {
        let main = SwarmId::main();
        let limiter = SendLimiter::new(1);
        let t0 = Time::from_parts(2026, 1, 1, 0, 0, 0).unwrap();
        assert!(limiter.allow(&main, "a", t0));
        assert!(!limiter.allow(&main, "a", t0));
        limiter.refund(&main, "a");
        assert!(limiter.allow(&main, "a", t0));
        // Refunding an agent with no bucket does nothing.
        limiter.refund(&main, "b");
    }

    #[test]
    fn registrations_share_one_bucket() {
        let limiter = RegisterLimiter::new(2);
        let t0 = Time::from_parts(2026, 1, 1, 0, 0, 0).unwrap();
        assert!(limiter.allow(t0));
        assert!(limiter.allow(t0));
        assert!(!limiter.allow(t0));
        limiter.refund();
        assert!(limiter.allow(t0));
        assert!(!limiter.allow(t0));
        // Half a minute refills one.
        assert!(limiter.allow(t0 + Span::seconds(30)));
    }
}
