//! kkt-container's general `Projects` over Docker Compose: `Docker` runs
//! each operation as the `docker compose` command for it, in the project's
//! folder (in `projects`). It knows nothing of what runs in the project.
//! Every command and file goes through the private runner, so it is tested
//! against a fake host.

mod projects;
mod runner;
#[cfg(feature = "testing")]
pub mod testing;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use runner::OsHost;

/// Private machine operations used by Docker and its tests.
trait Runner {
    /// Runs `command` and collects its output.
    fn run(&self, command: &Command) -> io::Result<Finished>;
    /// Runs `command` with its output going straight to the terminal, as for
    /// a download someone is waiting on. The returned output is empty.
    fn run_attached(&self, command: &Command) -> io::Result<Finished>;
    fn exists(&self, path: &Path) -> bool;
    /// Whether this path is a regular file, following links.
    fn is_file(&self, path: &Path) -> bool;
    fn read_file(&self, path: &Path) -> io::Result<String>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    /// Writes `contents`, replacing any file there. With `private` only the
    /// owner may read it, whether or not the file existed.
    fn write_file(&self, path: &Path, contents: &str, private: bool) -> io::Result<()>;
}

/// A program to run and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Command {
    pub program: &'static str,
    pub args: Vec<String>,
    /// Environment variables the command must run without.
    pub unset: Vec<String>,
    /// The folder to run in. Compose finds the files `COMPOSE_FILE` names
    /// relative to where it runs, not to `--project-directory`, so Compose
    /// commands run in the project's folder.
    pub current_dir: Option<PathBuf>,
}

/// Shown the way a person would type it, for messages.
impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for arg in &self.args {
            if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '"') {
                write!(f, " {arg:?}")?;
            } else {
                write!(f, " {arg}")?;
            }
        }
        Ok(())
    }
}

/// How a command ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Finished {
    /// The exit status, or `None` if a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Docker and Compose, implementing kkt-container's `Projects` directly.
pub struct Docker {
    /// The machine the commands run on and the files are written to.
    host: Box<dyn Runner>,
}

impl Docker {
    /// Docker and Compose on this machine.
    pub fn new() -> Docker {
        Docker {
            host: Box::new(OsHost),
        }
    }

    /// Docker and Compose over a scripted host instead of this machine,
    /// for tests: `host` answers every command and keeps a transcript.
    #[cfg(feature = "testing")]
    pub fn scripted(host: testing::Scripted) -> Docker {
        Docker {
            host: Box::new(host),
        }
    }
}

impl Default for Docker {
    /// Docker and Compose on this machine, as [`Docker::new`].
    fn default() -> Docker {
        Docker::new()
    }
}
