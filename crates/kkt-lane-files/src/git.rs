//! A lane folder's git commit, from the answers of the `git` commands that
//! find it. Running a command is behind [`Git`], so what each answer means
//! is tested against a fake; the program supplies the real runner.
//!
//! Only an expected absence is "no commit": the folder is in no git
//! repository, the repository has no commit yet, git ignores the folder,
//! or git is not installed. Any other failure is an error carrying git's
//! own message.

use std::io;
use std::path::Path;

/// The commit checked out, if any. With `--verify --quiet`, a repository
/// with no commit yet exits 1 and prints nothing.
const REV_PARSE: &[&str] = &["rev-parse", "--verify", "--quiet", "HEAD"];
/// Exits 0 when git ignores the folder and 1 when it does not.
const CHECK_IGNORE: &[&str] = &["check-ignore", "--quiet", "."];
/// The folder's changes that the commit does not have.
const STATUS: &[&str] = &["status", "--porcelain", "--", "."];

/// The exit status git gives every fatal error, "not a git repository"
/// among them.
const FATAL: i32 = 128;
/// Git's message when the folder is in no repository. The runner must run
/// git in the C locale, so this text is not translated.
const NOT_A_REPOSITORY: &str = "not a git repository";

/// What one `git` command gave back.
pub struct Ran {
    /// `None` when it was stopped by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs git.
pub trait Git {
    /// Runs `git -C <folder> <args>` in the C locale. An error of kind
    /// `NotFound` means git is not installed.
    fn run(&self, folder: &Path, args: &[&str]) -> io::Result<Ran>;
}

/// The commit checked out where `folder` is, with `-dirty` added when files
/// in `folder` differ from it; `None` for an expected absence (see the
/// module's notes).
pub fn commit(git: &dyn Git, folder: &Path) -> io::Result<Option<String>> {
    let head = match git.run(folder, REV_PARSE) {
        Ok(ran) => ran,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    match head.code {
        Some(0) => {}
        Some(1) => return Ok(None),
        Some(FATAL) if head.stderr.contains(NOT_A_REPOSITORY) => return Ok(None),
        _ => return Err(failed(REV_PARSE, &head)),
    }
    let head = head.stdout.trim();
    if head.is_empty() {
        return Ok(None);
    }
    let ignored = git.run(folder, CHECK_IGNORE)?;
    match ignored.code {
        Some(0) => return Ok(None),
        Some(1) => {}
        _ => return Err(failed(CHECK_IGNORE, &ignored)),
    }
    let changes = git.run(folder, STATUS)?;
    if changes.code != Some(0) {
        return Err(failed(STATUS, &changes));
    }
    Ok(Some(if changes.stdout.trim().is_empty() {
        head.to_string()
    } else {
        format!("{head}-dirty")
    }))
}

/// The error for a command that failed: the command, its exit status and
/// git's message.
fn failed(args: &[&str], ran: &Ran) -> io::Error {
    let status = match ran.code {
        Some(code) => format!("exit status {code}"),
        None => "stopped by a signal".to_string(),
    };
    io::Error::other(format!(
        "`git {}` failed ({status}): {}",
        args.join(" "),
        ran.stderr.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Answers each command by its first argument; a command with no answer
    /// fails the test. Records the commands run.
    #[derive(Default)]
    struct FakeGit {
        answers: HashMap<&'static str, (Option<i32>, &'static str, &'static str)>,
        not_installed: bool,
        ran: RefCell<Vec<String>>,
    }

    impl FakeGit {
        fn answer(mut self, command: &'static str, code: i32, stdout: &'static str) -> Self {
            self.answers.insert(command, (Some(code), stdout, ""));
            self
        }

        fn fail(mut self, command: &'static str, code: i32, stderr: &'static str) -> Self {
            self.answers.insert(command, (Some(code), "", stderr));
            self
        }
    }

    impl Git for FakeGit {
        fn run(&self, folder: &Path, args: &[&str]) -> io::Result<Ran> {
            assert_eq!(folder, Path::new("/lanes"));
            self.ran.borrow_mut().push(args.join(" "));
            if self.not_installed {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            let (code, stdout, stderr) = self.answers[args[0]];
            Ok(Ran {
                code,
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            })
        }
    }

    const HEAD: &str = "4f1c2a9e7b3d5f60a8c1e2d3b4a5968778695a4b";

    fn in_checkout() -> FakeGit {
        FakeGit::default()
            .answer("rev-parse", 0, "4f1c2a9e7b3d5f60a8c1e2d3b4a5968778695a4b\n")
            .answer("check-ignore", 1, "")
    }

    fn commit_of(git: &FakeGit) -> io::Result<Option<String>> {
        commit(git, Path::new("/lanes"))
    }

    fn error_of(git: &FakeGit) -> String {
        commit_of(git).expect_err("an error").to_string()
    }

    #[test]
    fn a_clean_folder_has_the_commit_and_a_changed_one_is_dirty() {
        let git = in_checkout().answer("status", 0, "");
        assert_eq!(commit_of(&git).unwrap(), Some(HEAD.to_string()));
        assert_eq!(
            *git.ran.borrow(),
            [
                "rev-parse --verify --quiet HEAD",
                "check-ignore --quiet .",
                "status --porcelain -- ."
            ]
        );

        let git = in_checkout().answer("status", 0, " M reviewer.md\n");
        assert_eq!(commit_of(&git).unwrap(), Some(format!("{HEAD}-dirty")));
    }

    #[test]
    fn a_folder_outside_git_has_no_commit() {
        let git = FakeGit::default().fail(
            "rev-parse",
            128,
            "fatal: not a git repository (or any of the parent directories): .git\n",
        );
        assert_eq!(commit_of(&git).unwrap(), None);
        assert_eq!(git.ran.borrow().len(), 1);
    }

    #[test]
    fn a_repository_with_no_commit_yet_has_none() {
        let git = FakeGit::default().answer("rev-parse", 1, "");
        assert_eq!(commit_of(&git).unwrap(), None);
    }

    #[test]
    fn without_git_installed_there_is_no_commit() {
        let git = FakeGit {
            not_installed: true,
            ..FakeGit::default()
        };
        assert_eq!(commit_of(&git).unwrap(), None);
    }

    #[test]
    fn a_folder_git_ignores_has_no_commit() {
        let git = FakeGit::default()
            .answer("rev-parse", 0, HEAD)
            .answer("check-ignore", 0, "");
        assert_eq!(commit_of(&git).unwrap(), None);
        assert_eq!(git.ran.borrow().len(), 2);
    }

    #[test]
    fn a_failed_rev_parse_is_an_error_with_git_s_message() {
        let git = FakeGit::default().fail(
            "rev-parse",
            128,
            "fatal: detected dubious ownership in repository at '/lanes'\n",
        );
        assert_eq!(
            error_of(&git),
            "`git rev-parse --verify --quiet HEAD` failed (exit status 128): \
             fatal: detected dubious ownership in repository at '/lanes'"
        );
    }

    #[test]
    fn a_failed_check_ignore_is_an_error_not_an_answer() {
        let git = FakeGit::default().answer("rev-parse", 0, HEAD).fail(
            "check-ignore",
            128,
            "fatal: index file smaller than expected\n",
        );
        assert_eq!(
            error_of(&git),
            "`git check-ignore --quiet .` failed (exit status 128): \
             fatal: index file smaller than expected"
        );
        assert_eq!(git.ran.borrow().len(), 2);
    }

    #[test]
    fn a_failed_status_is_an_error_not_a_dirty_commit() {
        let git = in_checkout().fail("status", 128, "fatal: index file corrupt\n");
        assert_eq!(
            error_of(&git),
            "`git status --porcelain -- .` failed (exit status 128): fatal: index file corrupt"
        );
    }

    #[test]
    fn a_command_stopped_by_a_signal_is_an_error() {
        let mut git = in_checkout();
        git.answers.insert("status", (None, "", ""));
        assert_eq!(
            error_of(&git),
            "`git status --porcelain -- .` failed (stopped by a signal): "
        );
    }
}
