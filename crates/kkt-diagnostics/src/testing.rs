//! A prober in memory, a stand-in `Prober` for tests: key files and what
//! each address answers are given up front, and nothing touches the machine
//! or a network.

use crate::{Answered, Initialized, KeyFile, Prober, Probing};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Key files and servers, held in memory.
#[derive(Default)]
pub struct Memory {
    keys: BTreeMap<PathBuf, KeyFile>,
    servers: BTreeMap<String, (Answered, Initialized)>,
}

impl Memory {
    /// No key files and no servers: every key file is missing, and nothing
    /// answers at any address.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same, with `key` in the file at `path`, of permissions `mode`.
    pub fn key(mut self, path: impl Into<PathBuf>, mode: Option<u32>, key: &str) -> Self {
        self.keys.insert(
            path.into(),
            KeyFile {
                mode,
                text: Ok(key.to_string()),
            },
        );
        self
    }

    /// The same, with a server at `url` that answers a GET with `answered`
    /// and an `initialize` with `initialized`.
    pub fn server(mut self, url: &str, answered: Answered, initialized: Initialized) -> Self {
        self.servers
            .insert(url.to_string(), (answered, initialized));
        self
    }
}

impl Prober for Memory {
    fn key_file(&self, path: &Path) -> Result<KeyFile, String> {
        self.keys
            .get(path)
            .cloned()
            .ok_or_else(|| "No such file or directory (os error 2)".to_string())
    }

    fn answer<'a>(&'a self, url: &'a str) -> Probing<'a, Answered> {
        let answered = match self.servers.get(url) {
            Some((answered, _)) => answered.clone(),
            None => Err(format!("nothing answers at {url}")),
        };
        Box::pin(async move { Ok(answered) })
    }

    fn initialize<'a>(&'a self, url: &'a str, _: Option<&'a str>) -> Probing<'a, Initialized> {
        let initialized = match self.servers.get(url) {
            Some((_, initialized)) => initialized.clone(),
            None => Initialized::Failed(format!("nothing answers at {url}")),
        };
        Box::pin(async move { Ok(initialized) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Status;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    fn now<F: Future>(future: F) -> F::Output {
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("the stand-in never waits"),
        }
    }

    #[test]
    fn it_answers_what_it_was_given_and_nothing_else() {
        let ok = Status {
            code: 200,
            reason: Some("OK".into()),
        };
        let refused = Initialized::Refused(Status {
            code: 401,
            reason: Some("Unauthorized".into()),
        });
        let prober = Memory::new()
            .key("/k/access-key", Some(0o600), "a-key")
            .server("http://s/mcp", Ok(ok.clone()), refused.clone());
        let key = prober.key_file(Path::new("/k/access-key")).expect("found");
        assert_eq!((key.mode, key.text), (Some(0o600), Ok("a-key".to_string())));
        assert!(prober.key_file(Path::new("/k/other")).is_err());
        assert_eq!(now(prober.answer("http://s/mcp")), Ok(Ok(ok)));
        assert_eq!(now(prober.initialize("http://s/mcp", None)), Ok(refused));
        assert!(matches!(now(prober.answer("http://other/mcp")), Ok(Err(_))));
    }
}
