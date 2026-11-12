//! A stand-in server in memory, for tests of the crates that use this
//! contract. It registers and reclaims avatars and keeps their
//! descriptions; a watch of it sees the watcher's handle, no channels and no
//! messages, and a wait returns at once with nothing new. Any address
//! reaches the same server.

use super::{
    Address, Avatars, Channel, Failure, Message, Reclaimed, Registered, Servers, Waited, Watched,
};
use futures::future::BoxFuture;
use kkt_clock::Clock;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// The admin's name as a watch sees it.
pub const ADMIN_HANDLE: &str = "(admin)";

/// A server in memory. Clones share it.
#[derive(Clone, Default)]
pub struct Memory {
    state: Arc<Mutex<State>>,
}

/// The avatars, and how many tokens have been issued: each token is new.
#[derive(Default)]
struct State {
    avatars: Vec<Avatar>,
    issued: usize,
}

impl State {
    /// A token never issued before.
    fn token(&mut self) -> String {
        self.issued += 1;
        format!("act_{}", self.issued)
    }
}

/// One registered avatar, as `Memory` keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avatar {
    pub handle: String,
    pub token: String,
    pub recovery_code: String,
    pub lane: Option<String>,
    pub description: Option<String>,
}

impl Memory {
    pub fn new() -> Memory {
        Memory::default()
    }

    /// Every avatar registered so far, in order.
    pub fn registered(&self) -> Vec<Avatar> {
        self.lock().avatars.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A test that panicked while holding the lock has already failed.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn refused(code: &str, message: &str) -> Failure {
    Failure::Refused {
        code: code.into(),
        message: message.into(),
    }
}

impl Servers for Memory {
    fn avatars(&self, _: Address) -> Box<dyn Avatars> {
        Box::new(self.clone())
    }
    fn agent(&self, _: Address, token: String) -> Box<dyn Watched> {
        Box::new(Watch {
            server: self.clone(),
            token: Some(token),
        })
    }
    fn admin(&self, _: Address, _: Arc<dyn Clock>) -> Box<dyn Watched> {
        Box::new(Watch {
            server: self.clone(),
            token: None,
        })
    }
}

impl Avatars for Memory {
    fn register<'a>(
        &'a self,
        handle: Option<&'a str>,
        lane: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Registered, Failure>> {
        Box::pin(async move {
            let mut state = self.lock();
            let n = state.avatars.len() + 1;
            let handle = handle.map_or_else(|| format!("agent-{n}"), str::to_string);
            if state.avatars.iter().any(|a| a.handle == handle) {
                return Err(refused("handle_taken", "that handle is taken"));
            }
            let avatar = Avatar {
                handle,
                token: state.token(),
                recovery_code: format!("rc_{n}"),
                lane: lane.map(str::to_string),
                description: None,
            };
            state.avatars.push(avatar.clone());
            Ok(Registered {
                handle: avatar.handle,
                token: avatar.token,
                recovery_code: avatar.recovery_code,
            })
        })
    }

    fn reclaim<'a>(
        &'a self,
        handle: &'a str,
        recovery_code: &'a str,
    ) -> BoxFuture<'a, Result<Reclaimed, Failure>> {
        Box::pin(async move {
            let mut state = self.lock();
            let i = state
                .avatars
                .iter()
                .position(|a| a.handle == handle && a.recovery_code == recovery_code)
                .ok_or_else(|| refused("unauthenticated", "wrong handle or recovery code"))?;
            // A new token, never issued before; the old one stops working.
            let token = state.token();
            let avatar = &mut state.avatars[i];
            avatar.token = token;
            Ok(Reclaimed {
                handle: avatar.handle.clone(),
                token: avatar.token.clone(),
            })
        })
    }

    fn describe<'a>(&'a self, token: &'a str, text: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let mut state = self.lock();
            let avatar = state
                .avatars
                .iter_mut()
                .find(|a| a.token == token)
                .ok_or_else(|| refused("unauthenticated", "unknown token"))?;
            avatar.description = Some(text.to_string());
            Ok(())
        })
    }
}

/// A watch of `Memory`: as the agent `token` names, or as the admin.
struct Watch {
    server: Memory,
    token: Option<String>,
}

impl Watched for Watch {
    fn handle(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move {
            match &self.token {
                None => Ok(ADMIN_HANDLE.to_string()),
                Some(token) => self
                    .server
                    .lock()
                    .avatars
                    .iter()
                    .find(|a| &a.token == token)
                    .map(|a| a.handle.clone())
                    .ok_or_else(|| refused("unauthenticated", "unknown token")),
            }
        })
    }
    fn channels(&self) -> BoxFuture<'_, Result<Vec<Channel>, Failure>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn latest_id(&self) -> BoxFuture<'_, Result<i64, Failure>> {
        Box::pin(async { Ok(0) })
    }
    fn message(&self, _: i64) -> BoxFuture<'_, Result<Message, Failure>> {
        Box::pin(async { Err(refused("not_found", "no such message")) })
    }
    fn wait<'a>(
        &'a self,
        _: i64,
        _: Option<&'a [String]>,
        _: Duration,
    ) -> BoxFuture<'a, Result<Waited, Failure>> {
        Box::pin(async { Ok(Waited::default()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    fn at() -> Address {
        Address {
            url: "http://127.0.0.1:7420/mcp".into(),
            key: None,
            key_name: None,
        }
    }

    #[test]
    fn a_registered_avatar_watches_as_itself_and_keeps_its_description() {
        let server = Memory::new();
        let avatars = server.avatars(at());
        let registered = block_on(avatars.register(Some("amber-falcon"), Some("docs")))
            .unwrap_or_else(|e| panic!("{e}"));
        block_on(avatars.describe(&registered.token, "writes docs")).expect("described");
        let watch = server.agent(at(), registered.token.clone());
        assert_eq!(block_on(watch.handle()), Ok("amber-falcon".to_string()));
        assert_eq!(
            server.registered(),
            [Avatar {
                handle: "amber-falcon".into(),
                token: registered.token,
                recovery_code: registered.recovery_code,
                lane: Some("docs".into()),
                description: Some("writes docs".into()),
            }]
        );
    }

    #[test]
    fn a_taken_handle_and_a_wrong_recovery_code_are_refused() {
        let server = Memory::new();
        let avatars = server.avatars(at());
        let first = block_on(avatars.register(Some("amber-falcon"), None))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            block_on(avatars.register(Some("amber-falcon"), None)),
            Err(Failure::Refused { code, .. }) if code == "handle_taken"
        ));
        assert!(block_on(avatars.reclaim("amber-falcon", "wrong")).is_err());
        let reclaimed = block_on(avatars.reclaim("amber-falcon", &first.recovery_code))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(reclaimed.token, first.token);
        let old = server.agent(at(), first.token);
        assert!(block_on(old.handle()).is_err());
    }

    #[test]
    fn each_reclaim_issues_a_token_no_other_avatar_or_earlier_reclaim_has() {
        let server = Memory::new();
        let avatars = server.avatars(at());
        let alice =
            block_on(avatars.register(Some("alice"), None)).unwrap_or_else(|e| panic!("{e}"));
        let bob = block_on(avatars.register(Some("bob"), None)).unwrap_or_else(|e| panic!("{e}"));
        let reclaim = |handle, code: &str| {
            block_on(avatars.reclaim(handle, code)).unwrap_or_else(|e| panic!("{e}"))
        };
        let alice_2 = reclaim("alice", &alice.recovery_code);
        let bob_2 = reclaim("bob", &bob.recovery_code);
        let alice_3 = reclaim("alice", &alice.recovery_code);
        let tokens = [
            &alice.token,
            &bob.token,
            &alice_2.token,
            &bob_2.token,
            &alice_3.token,
        ];
        for (i, a) in tokens.iter().enumerate() {
            for b in &tokens[i + 1..] {
                assert_ne!(a, b, "a token was issued twice");
            }
        }
        let watches_as = |token: &str| block_on(server.agent(at(), token.to_string()).handle());
        assert_eq!(watches_as(&bob_2.token), Ok("bob".to_string()));
        assert_eq!(watches_as(&alice_3.token), Ok("alice".to_string()));
        // Each reclaim retires the token before it.
        assert!(watches_as(&alice_2.token).is_err());
        assert!(watches_as(&alice.token).is_err());
        assert!(watches_as(&bob.token).is_err());
    }

    #[test]
    fn a_watch_of_it_finds_nothing_new() {
        let server = Memory::new();
        struct NoClock;
        impl Clock for NoClock {
            fn now(&self) -> kkt_clock::Time {
                unreachable!("the stand-in keeps no time")
            }
            fn sleep_until(&self, _: kkt_clock::Time) -> BoxFuture<'_, ()> {
                unreachable!("the stand-in keeps no time")
            }
        }
        let watch = server.admin(at(), Arc::new(NoClock));
        assert_eq!(block_on(watch.handle()), Ok(ADMIN_HANDLE.to_string()));
        assert_eq!(block_on(watch.latest_id()), Ok(0));
        assert_eq!(
            block_on(watch.wait(0, None, Duration::from_secs(60))),
            Ok(Waited::default())
        );
    }
}
