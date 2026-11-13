//! The browser with the system's own opener: `open` on macOS, `xdg-open`
//! elsewhere. It starts the opener with the link and does not wait for it;
//! a thread waits for it and reaps it, and a failure is kept to report once.
//! Which opener, and how a failure is worded, are plain functions tested
//! without `real`; starting the opener is behind `real`.

use std::sync::{Mutex, PoisonError};

/// The system's opener, which the program injects as the `Browser`.
#[cfg(feature = "real")]
pub struct SystemBrowser {
    opener: String,
    failure: std::sync::Arc<Mutex<Option<String>>>,
}

#[cfg(feature = "real")]
impl SystemBrowser {
    #[allow(clippy::new_without_default)]
    pub fn new() -> SystemBrowser {
        SystemBrowser::with_opener(system_opener())
    }

    /// A browser that opens with `opener`, a program that takes the link as
    /// its only argument.
    fn with_opener(opener: impl Into<String>) -> SystemBrowser {
        SystemBrowser {
            opener: opener.into(),
            failure: std::sync::Arc::new(Mutex::new(None)),
        }
    }
}

#[cfg(feature = "real")]
impl crate::Browser for SystemBrowser {
    /// Starts the system opener without waiting for it to finish.
    fn open(&self, url: &str) -> std::io::Result<()> {
        use std::process::Stdio;
        let mut child = std::process::Command::new(&self.opener)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        // The opener is waited for on its own thread, which also reaps it:
        // the caller does not wait, and learns later if it failed.
        let failure = std::sync::Arc::clone(&self.failure);
        std::thread::spawn(move || {
            let ended = child
                .wait()
                .map(|status| (status.success(), status.to_string()));
            if let Some(why) = failure_text(ended) {
                record_failure(&failure, why);
            }
        });
        Ok(())
    }

    /// Takes a completed opener failure, once.
    fn open_failure(&self) -> Option<String> {
        take_failure(&self.failure)
    }
}

/// What to say of an opener that ended: nothing if it succeeded, else its
/// exit status, or why it could not be waited for. `ended` is whether it
/// succeeded and its status as the system words it.
fn failure_text(ended: std::io::Result<(bool, String)>) -> Option<String> {
    match ended {
        Ok((true, _)) => None,
        Ok((false, status)) => Some(format!("the opener ended with {status}")),
        Err(e) => Some(format!("could not wait for the opener: {e}")),
    }
}

/// Keeps `why` for `open_failure`. A poisoned slot still takes it and stays
/// poisoned, so that `open_failure` reports the poisoning with it.
fn record_failure(failure: &Mutex<Option<String>>, why: String) {
    *failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
}

/// Takes a kept failure, once.
fn take_failure(failure: &Mutex<Option<String>>) -> Option<String> {
    match failure.lock() {
        Ok(mut slot) => slot.take(),
        // The slot holds only a finished message, so a thread that panicked
        // while holding it cannot have left it half written: what it holds
        // is still reported, once, with the poisoning as its cause. Until
        // there is a failure the slot stays poisoned, so the cause goes with
        // the failure it came before.
        Err(poisoned) => {
            let cause = poisoned.to_string();
            let why = poisoned.into_inner().take()?;
            failure.clear_poison();
            Some(format!("{why} ({cause})"))
        }
    }
}

/// The program that opens a link in the person's browser.
fn system_opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

#[cfg(test)]
mod tests;
