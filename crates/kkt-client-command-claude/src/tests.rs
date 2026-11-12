//! The arguments and the wording, over a fake runner.

use super::*;
use std::cell::RefCell;
use std::path::PathBuf;

/// Records each run and answers with `answer`.
struct Fake {
    runs: RefCell<Vec<(Vec<String>, Option<PathBuf>)>>,
    answer: fn() -> io::Result<Ran>,
}

impl Fake {
    fn new(answer: fn() -> io::Result<Ran>) -> Fake {
        Fake {
            runs: RefCell::new(Vec::new()),
            answer,
        }
    }
}

impl Runner for Fake {
    fn run(&self, args: &[&str], config_dir: Option<&Path>) -> io::Result<Ran> {
        self.runs.borrow_mut().push((
            args.iter().map(|a| a.to_string()).collect(),
            config_dir.map(Path::to_path_buf),
        ));
        (self.answer)()
    }
}

fn succeeds() -> io::Result<Ran> {
    Ok(Ran {
        success: true,
        status: "exit status: 0".into(),
    })
}

#[test]
fn removing_and_adding_ask_claude_mcp_at_user_scope_under_kkt() {
    let fake = Fake::new(succeeds);
    remove_server(&fake, None).expect("removed");
    let dir = Path::new("/profiles/work");
    add_server(&fake, r#"{"type":"http"}"#, Some(dir)).expect("added");
    assert_eq!(
        *fake.runs.borrow(),
        [
            (
                vec!["mcp", "remove", "--scope", "user", "kkt"]
                    .into_iter()
                    .map(String::from)
                    .collect::<Vec<_>>(),
                None
            ),
            (
                vec![
                    "mcp",
                    "add-json",
                    "--scope",
                    "user",
                    "kkt",
                    r#"{"type":"http"}"#
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                Some(dir.to_path_buf())
            ),
        ]
    );
}

#[test]
fn a_failed_run_says_only_its_status() {
    let fake = Fake::new(|| {
        Ok(Ran {
            success: false,
            status: "exit status: 1".into(),
        })
    });
    assert_eq!(
        remove_server(&fake, None),
        Err("claude mcp failed with exit status: 1".to_string())
    );
}

#[test]
fn a_missing_claude_says_how_to_get_it_and_other_failures_say_why() {
    let missing = Fake::new(|| Err(io::Error::from(io::ErrorKind::NotFound)));
    assert_eq!(
        add_server(&missing, "{}", None),
        Err("claude was not found on PATH; install Claude Code, then retry --write".to_string())
    );
    let denied = Fake::new(|| Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")));
    assert_eq!(
        add_server(&denied, "{}", None),
        Err("could not run claude: denied".to_string())
    );
}
