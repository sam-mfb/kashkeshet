//! A scripted host for tests of what `Docker` does: [`Scripted`] answers
//! every command from a script, keeps files and folders in memory, and
//! writes down, in order, everything `Docker` did through it, as a
//! transcript a test can compare with a golden file.

use super::{Command, Finished, Runner};
use kkt_clock::Span;
use kkt_clock::testing::FixedClock;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// How a scripted command ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    /// The exit status, or `None` if a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Reply {
    /// Exit 0, printing `stdout`.
    pub fn ok(stdout: &str) -> Reply {
        Reply::exit(0, stdout, "")
    }

    /// Exit `code`, printing `stdout` and `stderr`.
    pub fn exit(code: i32, stdout: &str, stderr: &str) -> Reply {
        Reply {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    /// Ended by a signal, printing `stdout`.
    pub fn signal(stdout: &str) -> Reply {
        Reply {
            code: None,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }
}

/// How the host answers a command, given the command as a person would
/// type it. An error means the command could not be run at all.
type Script = Box<dyn Fn(&str) -> io::Result<Reply>>;

/// A host for `Docker` (through [`super::Docker::scripted`]) whose commands
/// are answered by a script. Every command moves the clock it is given on
/// by three seconds, longer than the poll interval, so waiting never really
/// waits. Clones share one host.
#[derive(Clone)]
pub struct Scripted(Rc<Inner>);

struct Inner {
    clock: Arc<FixedClock>,
    script: Script,
    working_dir: Option<PathBuf>,
    files: RefCell<BTreeMap<PathBuf, String>>,
    dirs: RefCell<BTreeSet<PathBuf>>,
    /// Writes to these paths fail once this many have been made.
    failing_writes: BTreeMap<PathBuf, usize>,
    writes_made: RefCell<BTreeMap<PathBuf, usize>>,
    failing_dirs: BTreeSet<PathBuf>,
    events: RefCell<Vec<String>>,
    /// The variables the last command shown ran without.
    last_unset: RefCell<Option<Vec<String>>>,
    /// Texts a write is shown as a name instead of line by line.
    known: RefCell<Vec<(String, String)>>,
    /// Texts an answer shows as a marker instead.
    masked: RefCell<Vec<(String, String)>>,
}

impl Scripted {
    /// A host answering commands with `script` and moving `clock` on as
    /// they run. The person runs kkt in `/home/me`.
    pub fn new(
        clock: Arc<FixedClock>,
        script: impl Fn(&str) -> io::Result<Reply> + 'static,
    ) -> Scripted {
        Scripted(Rc::new(Inner {
            clock,
            script: Box::new(script),
            working_dir: Some(PathBuf::from("/home/me")),
            files: RefCell::default(),
            dirs: RefCell::default(),
            failing_writes: BTreeMap::new(),
            writes_made: RefCell::default(),
            failing_dirs: BTreeSet::new(),
            events: RefCell::default(),
            last_unset: RefCell::default(),
            known: RefCell::default(),
            masked: RefCell::default(),
        }))
    }

    fn inner_mut(&mut self) -> &mut Inner {
        Rc::get_mut(&mut self.0).expect("set a scripted host up before cloning it")
    }

    /// With a file at `path` holding `text`, and its folder.
    pub fn with_file(mut self, path: impl Into<PathBuf>, text: &str) -> Scripted {
        let path = path.into();
        let inner = self.inner_mut();
        if let Some(parent) = path.parent() {
            inner.dirs.get_mut().insert(parent.to_path_buf());
        }
        inner.files.get_mut().insert(path, text.to_string());
        self
    }

    /// With a folder at `path`.
    pub fn with_dir(mut self, path: impl Into<PathBuf>) -> Scripted {
        self.inner_mut().dirs.get_mut().insert(path.into());
        self
    }

    /// Writes to `path` fail once `after` have been made.
    pub fn failing_write(mut self, path: impl Into<PathBuf>, after: usize) -> Scripted {
        self.inner_mut().failing_writes.insert(path.into(), after);
        self
    }

    /// Making the folder `path` fails.
    pub fn failing_dir(mut self, path: impl Into<PathBuf>) -> Scripted {
        self.inner_mut().failing_dirs.insert(path.into());
        self
    }

    /// The folder the person runs kkt in cannot be found.
    pub fn without_working_dir(mut self) -> Scripted {
        self.inner_mut().working_dir = None;
        self
    }

    /// A write of exactly `text` is shown as `name`, not line by line.
    pub fn knowing(self, name: &str, text: &str) -> Scripted {
        self.0
            .known
            .borrow_mut()
            .push((name.to_string(), text.to_string()));
        self
    }

    /// Wherever `text` is in a command's answer, the transcript shows
    /// `<marker>` instead. Files written still show it as it is.
    pub fn masking(self, marker: &str, text: &str) -> Scripted {
        self.0
            .masked
            .borrow_mut()
            .push((format!("<{marker}>"), text.to_string()));
        self
    }

    /// Adds a line of the test's own to the transcript, such as what a
    /// stand-in login link was shown.
    pub fn note(&self, line: &str) {
        self.0.events.borrow_mut().push(line.to_string());
    }

    /// Everything done through this host, a line each, in order, with the
    /// script's answer under each command (`->`: its exit status and what
    /// it printed, or why it could not run). An entry repeated straight
    /// after itself is shown once, with how many times.
    pub fn transcript(&self) -> String {
        let events = self.0.events.borrow();
        let mut out = String::new();
        let mut index = 0;
        while index < events.len() {
            let line = &events[index];
            let mut times = 1;
            while events.get(index + times) == Some(line) {
                times += 1;
            }
            out.push_str(line);
            if times > 1 {
                out.push_str(&format!("  (x{times})"));
            }
            out.push('\n');
            index += times;
        }
        out
    }

    fn command(&self, kind: &str, command: &Command) -> io::Result<Finished> {
        let mut unset = command.unset.clone();
        unset.sort();
        if self.0.last_unset.borrow().as_ref() != Some(&unset) {
            self.note(&format!("{:<8} {}", "unset", unset.join(" ")));
            *self.0.last_unset.borrow_mut() = Some(unset);
        }
        let text = command.to_string();
        let cwd = command
            .current_dir
            .as_ref()
            .map(|dir| format!("cwd={}  ", dir.display()))
            .unwrap_or_default();
        self.0.clock.advance(Span::seconds(3));
        let answered = (self.0.script)(&text);
        // The command and the host's answer are one entry, so a command
        // answered the same way again and again is shown once.
        let answer = match &answered {
            Ok(reply) => {
                let mut answer = match reply.code {
                    Some(code) => format!("  -> exit {code}"),
                    None => "  -> ended by a signal".to_string(),
                };
                if !reply.stdout.is_empty() {
                    answer.push_str(&format!(", out {:?}", reply.stdout));
                }
                if !reply.stderr.is_empty() {
                    answer.push_str(&format!(", err {:?}", reply.stderr));
                }
                answer
            }
            Err(e) => format!("  -> cannot run: {e}"),
        };
        let answer = self
            .0
            .masked
            .borrow()
            .iter()
            .fold(answer, |answer, (marker, text)| {
                answer.replace(text, marker)
            });
        self.note(&format!("{kind:<8} {cwd}{text}\n{answer}"));
        let reply = answered?;
        Ok(Finished {
            code: reply.code,
            stdout: reply.stdout,
            stderr: reply.stderr,
        })
    }
}

impl Runner for Scripted {
    fn run(&self, command: &Command) -> io::Result<Finished> {
        self.command("run", command)
    }

    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        self.command("attached", command)
    }

    fn exists(&self, path: &Path) -> bool {
        self.0.files.borrow().contains_key(path) || self.0.dirs.borrow().contains(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        self.0.files.borrow().contains_key(path)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        self.0
            .files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.note(&format!("{:<8} {}", "mkdir", path.display()));
        if self.0.failing_dirs.contains(path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "not allowed",
            ));
        }
        let mut dirs = self.0.dirs.borrow_mut();
        for dir in path.ancestors() {
            dirs.insert(dir.to_path_buf());
        }
        Ok(())
    }

    fn write_file(&self, path: &Path, contents: &str, private: bool) -> io::Result<()> {
        let mode = if private { "private" } else { "plain" };
        self.note(&format!("{:<8} {mode} {}", "write", path.display()));
        let made = *self.0.writes_made.borrow().get(path).unwrap_or(&0);
        if self
            .0
            .failing_writes
            .get(path)
            .is_some_and(|after| made >= *after)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "not allowed",
            ));
        }
        // Like a real disk: the folder a file goes in must be there.
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if !self.0.dirs.borrow().contains(&parent) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such folder"));
        }
        let known = self
            .0
            .known
            .borrow()
            .iter()
            .find(|(_, text)| text == contents)
            .map(|(name, _)| name.clone());
        match known {
            Some(name) => self.note(&format!("  = {name}")),
            None => {
                for line in contents.lines() {
                    self.note(&format!("  | {line}"));
                }
                if !contents.is_empty() && !contents.ends_with('\n') {
                    self.note("  (no newline at the end)");
                }
            }
        }
        self.0
            .writes_made
            .borrow_mut()
            .insert(path.to_path_buf(), made + 1);
        self.0
            .files
            .borrow_mut()
            .insert(path.to_path_buf(), contents.to_string());
        Ok(())
    }

    fn say(&self, line: &str) {
        self.note(&format!("{:<8} {line}", "say"));
    }

    fn working_dir(&self) -> io::Result<PathBuf> {
        self.0
            .working_dir
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no current folder"))
    }
}
