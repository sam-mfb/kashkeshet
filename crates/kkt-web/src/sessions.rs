//! Sessions: who has signed in to the web page, and until when, and the
//! form tokens tied to them. Held in memory, so a restart signs everyone
//! out. A session holds what the [`Gate`](crate::Gate) said of the key its
//! holder signed in with, and nothing about the key itself.

use kkt_clock::{Clock, Time};
use kkt_entropy::Entropy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Characters in a web session ID, from the 36-character alphabet: about
/// 165 bits. The open page's form token is as long.
pub const WEB_SESSION_ID_CHARS: usize = 32;

/// Characters session IDs and form tokens are drawn from: lowercase ASCII
/// letters and digits, as before they moved here.
const RANDOM_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// What a session can hold: whatever the gate says a key admits. Any type
/// that can be cloned and sent between threads will do.
pub trait Held: Clone + Send + Sync + 'static {}

impl<T: Clone + Send + Sync + 'static> Held for T {}

/// Whether viewers sign in to see the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Every page is open, and there is no login form.
    Open,
    /// Viewers sign in with a key, at `/login`, and then hold a session.
    SignIn,
}

/// The session a request presents, as [`Sessions::session`] finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Session<T> {
    /// The request carries no session ID.
    Absent,
    /// The session ID was never issued, was signed out, or has expired.
    Ended,
    /// A live session, holding what the gate said at sign-in.
    Live(T),
}

/// The sessions in effect. Only a hash of each session ID is kept.
pub struct Sessions<T> {
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
    lifetime: Duration,
    access: Access,
    /// The form token when no sign-in is required, so there are no
    /// sessions to tie one to. Drawn when the sessions are made; a restart
    /// changes it.
    open_form_token: String,
    live: Mutex<HashMap<[u8; 32], Started<T>>>,
}

struct Started<T> {
    holder: T,
    started: Time,
}

impl<T: Held> Sessions<T> {
    /// Sessions that last `lifetime` from sign-in, timed by `clock`, with IDs
    /// and the open page's form token drawn from `entropy`, for a page with
    /// `access`.
    pub fn new(
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
        lifetime: Duration,
        access: Access,
    ) -> Sessions<T> {
        Sessions {
            open_form_token: entropy.random_chars(RANDOM_CHARS, WEB_SESSION_ID_CHARS),
            clock,
            entropy,
            lifetime,
            access,
            live: Mutex::new(HashMap::new()),
        }
    }

    /// Whether viewers sign in to see the page.
    pub fn access(&self) -> Access {
        self.access
    }

    /// Whether viewers must sign in: when not, every page is open and there
    /// is no login form.
    pub fn sign_in_required(&self) -> bool {
        self.access == Access::SignIn
    }

    /// How long a session lasts.
    pub fn lifetime(&self) -> Duration {
        self.lifetime
    }

    /// Starts a session holding `holder` and returns its ID, which only the
    /// caller ever sees. Expired sessions are dropped first.
    pub fn start(&self, holder: T) -> String {
        let id = self
            .entropy
            .random_chars(RANDOM_CHARS, WEB_SESSION_ID_CHARS);
        let now = self.clock.now();
        let mut live = self.live();
        live.retain(|_, s| self.within_lifetime(s, now));
        live.insert(
            hash(&id),
            Started {
                holder,
                started: now,
            },
        );
        id
    }

    /// The session with ID `id`, if the request carries one. An expired
    /// session is dropped.
    pub fn session(&self, id: Option<&str>) -> Session<T> {
        let Some(id) = id else {
            return Session::Absent;
        };
        let key = hash(id);
        let now = self.clock.now();
        let mut live = self.live();
        match live.get(&key) {
            Some(s) if self.within_lifetime(s, now) => Session::Live(s.holder.clone()),
            Some(_) => {
                live.remove(&key);
                Session::Ended
            }
            None => Session::Ended,
        }
    }

    /// Ends the session with ID `id`, if there is one.
    pub fn end(&self, id: &str) {
        self.live().remove(&hash(id));
    }

    /// The token a page puts in its forms, so a change can be told from a
    /// request forged by another site, which cannot read the page. With
    /// sign-in, it belongs to the viewer's live session (a hash of the
    /// session ID, which only the viewer's cookie holds); `None` without one.
    /// Without sign-in, it is one token for this server run.
    pub fn form_token(&self, session_id: Option<&str>) -> Option<String> {
        if !self.sign_in_required() {
            return Some(self.open_form_token.clone());
        }
        let id = session_id?;
        match self.session(Some(id)) {
            Session::Live(_) => {
                let digest = kkt_hash::sha256(format!("kkt-form:{id}").as_bytes());
                Some(digest.iter().map(|b| format!("{b:02x}")).collect())
            }
            Session::Absent | Session::Ended => None,
        }
    }

    /// Whether `presented` is the form token for the viewer holding
    /// `session_id`. Takes the same time whatever `presented` is.
    pub fn form_token_matches(&self, session_id: Option<&str>, presented: &str) -> bool {
        let Some(expected) = self.form_token(session_id) else {
            return false;
        };
        let expected: [u8; 32] = kkt_hash::sha256(expected.as_bytes());
        let presented: [u8; 32] = kkt_hash::sha256(presented.as_bytes());
        let diff = expected
            .iter()
            .zip(presented.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        std::hint::black_box(diff) == 0
    }

    fn within_lifetime(&self, session: &Started<T>, now: Time) -> bool {
        // A clock that moved backwards gives a negative age, counted as zero.
        let age = (now - session.started).to_std().unwrap_or(Duration::ZERO);
        age < self.lifetime
    }

    /// The session table. Each change to it is a single insert or removal,
    /// so a panic elsewhere cannot leave it half changed, and it stays usable
    /// after one.
    fn live(&self) -> MutexGuard<'_, HashMap<[u8; 32], Started<T>>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn hash(id: &str) -> [u8; 32] {
    kkt_hash::sha256(id.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_clock::Span;
    use kkt_clock::testing::FixedClock;
    use kkt_entropy::testing::SeqEntropy;

    fn sessions(lifetime_secs: u64, access: Access) -> (Arc<FixedClock>, Sessions<&'static str>) {
        sessions_with_seed(lifetime_secs, access, 1)
    }

    fn sessions_with_seed(
        lifetime_secs: u64,
        access: Access,
        seed: u64,
    ) -> (Arc<FixedClock>, Sessions<&'static str>) {
        let start = Time::from_parts(2026, 1, 1, 0, 0, 0).unwrap();
        let clock = Arc::new(FixedClock::new(start));
        let sessions = Sessions::new(
            clock.clone(),
            Arc::new(SeqEntropy::new(seed)),
            Duration::from_secs(lifetime_secs),
            access,
        );
        (clock, sessions)
    }

    #[test]
    fn a_started_session_is_live_until_its_lifetime_ends() {
        let (clock, sessions) = sessions(60, Access::SignIn);
        let id = sessions.start("web");
        assert_eq!(id.len(), WEB_SESSION_ID_CHARS);
        assert_eq!(sessions.session(Some(&id)), Session::Live("web"));
        clock.advance(Span::seconds(59));
        assert_eq!(sessions.session(Some(&id)), Session::Live("web"));
        clock.advance(Span::seconds(1));
        assert_eq!(sessions.session(Some(&id)), Session::Ended);
    }

    #[test]
    fn unknown_and_ended_sessions_are_not_live() {
        let (_, sessions) = sessions(60, Access::SignIn);
        assert_eq!(sessions.session(None), Session::Absent);
        assert_eq!(sessions.session(Some("never-issued")), Session::Ended);
        let id = sessions.start("web");
        let other = sessions.start("web");
        assert_ne!(id, other);
        sessions.end(&id);
        assert_eq!(sessions.session(Some(&id)), Session::Ended);
        assert_eq!(sessions.session(Some(&other)), Session::Live("web"));
    }

    #[test]
    fn expired_sessions_are_dropped_when_another_starts() {
        let (clock, sessions) = sessions(60, Access::SignIn);
        sessions.start("web");
        clock.advance(Span::seconds(60));
        sessions.start("web");
        assert_eq!(sessions.live().len(), 1);
    }

    #[test]
    fn only_a_hash_of_the_id_is_kept() {
        let (_, sessions) = sessions(60, Access::SignIn);
        let id = sessions.start("web");
        let live = sessions.live();
        let stored = live.keys().next().expect("one session");
        assert_eq!(stored, &hash(&id));
        assert_ne!(&stored[..], id.as_bytes());
    }

    #[test]
    fn form_tokens_belong_to_a_live_session() {
        let (clock, sessions) = sessions(3600, Access::SignIn);
        assert!(sessions.sign_in_required());
        assert_eq!(sessions.form_token(None), None);
        assert_eq!(sessions.form_token(Some("guess")), None);
        let id = sessions.start("web");
        let other = sessions.start("web");
        let token = sessions.form_token(Some(&id)).expect("a token");
        assert!(
            !token.contains(&id),
            "the token does not reveal the session"
        );
        assert!(sessions.form_token_matches(Some(&id), &token));
        // Another session's token, a wrong one, or none at all is refused.
        let others = sessions.form_token(Some(&other)).expect("a token");
        assert_ne!(token, others);
        assert!(!sessions.form_token_matches(Some(&id), &others));
        assert!(!sessions.form_token_matches(Some(&id), ""));
        assert!(!sessions.form_token_matches(None, &token));
        // It ends with its session.
        sessions.end(&other);
        assert!(!sessions.form_token_matches(Some(&other), &others));
        clock.advance(Span::seconds(3600));
        assert!(!sessions.form_token_matches(Some(&id), &token));
    }

    #[test]
    fn an_open_page_has_one_form_token_per_run() {
        let (_, sessions) = sessions_with_seed(3600, Access::Open, 7);
        assert!(!sessions.sign_in_required());
        let token = sessions.form_token(None).expect("a token");
        assert_eq!(token.len(), WEB_SESSION_ID_CHARS);
        assert_eq!(sessions.form_token(Some("anything")), Some(token.clone()));
        assert!(sessions.form_token_matches(None, &token));
        assert!(!sessions.form_token_matches(None, "wrong"));
        assert!(!sessions.form_token_matches(None, ""));
        let (_, restarted) = sessions_with_seed(3600, Access::Open, 8);
        assert_ne!(restarted.form_token(None), Some(token));
    }

    #[test]
    fn the_alphabet_is_lowercase_letters_and_digits() {
        let expected: Vec<u8> = (b'a'..=b'z').chain(b'0'..=b'9').collect();
        assert_eq!(RANDOM_CHARS, expected.as_slice());
    }
}
