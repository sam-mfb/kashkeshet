//! A prober in memory, for consumers' tests: key files and each address's
//! checks are given up front, and nothing touches the machine or a network.

use crate::{Check, KeyFile, Prober, Probing};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Key files and servers, held in memory.
#[derive(Default)]
pub struct Memory {
    keys: BTreeMap<PathBuf, KeyFile>,
    servers: BTreeMap<String, Result<Vec<Check>, String>>,
}

impl Memory {
    /// No key files and no servers: every key file is missing, and every
    /// probe fails.
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

    /// The same, with a probe of `url` giving `checks`, or the error.
    pub fn server(mut self, url: &str, checks: Result<Vec<Check>, String>) -> Self {
        self.servers.insert(url.to_string(), checks);
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

    fn probe<'a>(&'a self, url: &'a str, _: Option<&'a str>) -> Probing<'a> {
        let answer = self
            .servers
            .get(url)
            .cloned()
            .unwrap_or_else(|| Err(format!("nothing answers at {url}")));
        Box::pin(async move { answer })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let prober = Memory::new()
            .key("/k/access-key", Some(0o600), "a-key")
            .server("http://s/mcp", Ok(vec![Check::ok("server", "answered")]));
        let key = prober.key_file(Path::new("/k/access-key")).expect("found");
        assert_eq!((key.mode, key.text), (Some(0o600), Ok("a-key".to_string())));
        assert!(prober.key_file(Path::new("/k/other")).is_err());
        assert_eq!(
            now(prober.probe("http://s/mcp", None)),
            Ok(vec![Check::ok("server", "answered")])
        );
        assert!(now(prober.probe("http://other/mcp", None)).is_err());
    }
}
