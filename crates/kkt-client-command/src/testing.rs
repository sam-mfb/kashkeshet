//! A stand-in client command, for tests of the crates that ask one: it
//! records each call, and fails every call when told to.

use super::ClientCommand;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

/// One call, as `Recorder` keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Remove {
        config_dir: Option<PathBuf>,
    },
    Add {
        entry: String,
        config_dir: Option<PathBuf>,
    },
}

/// Records each call. With `fails` set, every call fails with "the client's
/// command failed" after it is recorded.
#[derive(Default)]
pub struct Recorder {
    pub calls: RefCell<Vec<Call>>,
    pub fails: Cell<bool>,
}

impl Recorder {
    fn answer(&self, call: Call) -> Result<(), String> {
        self.calls.borrow_mut().push(call);
        match self.fails.get() {
            true => Err("the client's command failed".into()),
            false => Ok(()),
        }
    }
}

impl ClientCommand for Recorder {
    fn remove_server(&self, config_dir: Option<&Path>) -> Result<(), String> {
        self.answer(Call::Remove {
            config_dir: config_dir.map(Path::to_path_buf),
        })
    }
    fn add_server(&self, entry: &str, config_dir: Option<&Path>) -> Result<(), String> {
        self.answer(Call::Add {
            entry: entry.to_string(),
            config_dir: config_dir.map(Path::to_path_buf),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_records_each_call_and_fails_them_when_told() {
        let recorder = Recorder::default();
        recorder.remove_server(None).expect("removed");
        recorder.fails.set(true);
        assert_eq!(
            recorder.add_server("{}", Some(Path::new("/profiles/work"))),
            Err("the client's command failed".to_string())
        );
        assert_eq!(
            *recorder.calls.borrow(),
            [
                Call::Remove { config_dir: None },
                Call::Add {
                    entry: "{}".into(),
                    config_dir: Some("/profiles/work".into())
                }
            ]
        );
    }
}
