//! The machine Docker runs on: files, commands and terminal lines.
//! Plain OsHost methods are adapted by the program to the container logic.

#[cfg(any(test, feature = "real"))]
use std::io;
use std::path::Path;
#[cfg(any(test, feature = "real"))]
use std::process::Stdio;

#[cfg(any(test, feature = "real"))]
pub struct OsHost;

/// A raw command and the environment and directory it runs with.
pub struct Command<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub unset: &'a [String],
    pub current_dir: Option<&'a Path>,
}

/// The machine's exit status and captured output.
#[derive(Default)]
pub struct Finished {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// The mode of private files written by this crate.
#[cfg(all(unix, any(test, feature = "real")))]
const PRIVATE_FILE_MODE: u32 = 0o600;

#[cfg(any(test, feature = "real"))]
impl OsHost {
    pub fn run(&self, command: &Command<'_>) -> io::Result<Finished> {
        let output = process(command).stdin(Stdio::null()).output()?;
        Ok(Finished {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    pub fn run_attached(&self, command: &Command<'_>) -> io::Result<Finished> {
        let status = process(command).stdin(Stdio::null()).status()?;
        Ok(Finished {
            code: status.code(),
            ..Finished::default()
        })
    }

    pub fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    pub fn read_file(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    pub fn write_file(&self, path: &Path, contents: &str, private: bool) -> io::Result<()> {
        write(path, contents, private)
    }

    pub fn say(&self, line: &str) {
        println!("{line}");
    }

    pub fn working_dir(&self) -> io::Result<std::path::PathBuf> {
        std::env::current_dir()
    }
}

/// The process for `command`: in its folder, without the variables it must
/// not see.
#[cfg(any(test, feature = "real"))]
fn process(command: &Command<'_>) -> std::process::Command {
    let mut process = std::process::Command::new(command.program);
    process.args(command.args);
    for name in command.unset {
        process.env_remove(name);
    }
    if let Some(dir) = command.current_dir {
        process.current_dir(dir);
    }
    process
}

#[cfg(unix)]
#[cfg(any(test, feature = "real"))]
fn write(path: &Path, contents: &str, private: bool) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mode = PRIVATE_FILE_MODE;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    if private {
        // New files start private, so there is no moment when they are not.
        options.mode(mode);
    }
    let mut file = options.open(path)?;
    if private {
        // A file that was already there keeps its old mode when opened.
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    file.write_all(contents.as_bytes())
}

#[cfg(not(unix))]
#[cfg(any(test, feature = "real"))]
fn write(path: &Path, contents: &str, _private: bool) -> io::Result<()> {
    std::fs::write(path, contents)
}

#[cfg(all(test, unix))]
use kkt_scratch as scratch;

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_private_file_is_made_private_even_over_a_looser_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::scratch::Dir::new("kkt-docker-host");
        let path = dir.join("key");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        OsHost.write_file(&path, "new", true).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(mode(&path), 0o600);
        let fresh = dir.join("fresh");
        OsHost.write_file(&fresh, "x", true).unwrap();
        assert_eq!(mode(&fresh), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn the_real_host_leaves_unset_variables_out_of_a_commands_environment() {
        // Cargo sets CARGO_MANIFEST_DIR for every test run.
        let print = |unset: Vec<String>| {
            OsHost
                .run(&Command {
                    program: "sh",
                    args: &[
                        "-c".into(),
                        "printf %s \"${CARGO_MANIFEST_DIR-unset}\"".into(),
                    ],
                    unset: &unset,
                    current_dir: None,
                })
                .unwrap()
                .stdout
        };
        assert_ne!(print(vec![]), "unset");
        assert_eq!(print(vec!["CARGO_MANIFEST_DIR".into()]), "unset");
    }

    #[cfg(unix)]
    #[test]
    fn the_real_host_runs_a_command_in_the_folder_it_names() {
        let dir = crate::scratch::Dir::new("kkt-docker-cwd");
        let here = OsHost
            .run(&Command {
                program: "pwd",
                args: &["-P".into()],
                unset: &[],
                current_dir: Some(&dir),
            })
            .unwrap()
            .stdout;
        assert_eq!(
            Path::new(here.trim()),
            std::fs::canonicalize(&dir).unwrap().as_path()
        );
    }
}
