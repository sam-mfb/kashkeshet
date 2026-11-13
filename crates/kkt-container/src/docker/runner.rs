//! Private native runner for the Docker wrapper.

#[cfg(any(test, feature = "real"))]
use std::io;
use std::path::Path;
#[cfg(any(test, feature = "real"))]
use std::process::Stdio;

#[cfg(any(test, feature = "real"))]
pub(super) struct OsHost;

/// A raw command and the environment and directory it runs with.
struct NativeCommand<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub unset: &'a [String],
    pub current_dir: Option<&'a Path>,
}

/// The machine's exit status and captured output.
#[derive(Default)]
struct NativeFinished {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// The mode of private files written by this crate.
#[cfg(all(unix, any(test, feature = "real")))]
const PRIVATE_FILE_MODE: u32 = 0o600;

#[cfg(any(test, feature = "real"))]
impl OsHost {
    fn run(&self, command: &NativeCommand<'_>) -> io::Result<NativeFinished> {
        let output = process(command).stdin(Stdio::null()).output()?;
        Ok(NativeFinished {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_attached(&self, command: &NativeCommand<'_>) -> io::Result<NativeFinished> {
        let status = process(command).stdin(Stdio::null()).status()?;
        Ok(NativeFinished {
            code: status.code(),
            ..NativeFinished::default()
        })
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn write_file(&self, path: &Path, contents: &str, private: bool) -> io::Result<()> {
        write(path, contents, private)
    }

    fn say(&self, line: &str) {
        println!("{line}");
    }

    fn working_dir(&self) -> io::Result<std::path::PathBuf> {
        std::env::current_dir()
    }
}

/// The process for `command`: in its folder, without the variables it must
/// not see.
#[cfg(any(test, feature = "real"))]
fn process(command: &NativeCommand<'_>) -> std::process::Command {
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
        let dir = scratch::Dir::new("kkt-docker-host");
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
                .run(&NativeCommand {
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
        let dir = scratch::Dir::new("kkt-docker-cwd");
        let here = OsHost
            .run(&NativeCommand {
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

#[cfg(any(test, feature = "real"))]
impl super::Runner for OsHost {
    fn run(&self, command: &super::Command) -> io::Result<super::Finished> {
        OsHost.run(&native_command(command)).map(finished)
    }

    fn run_attached(&self, command: &super::Command) -> io::Result<super::Finished> {
        OsHost.run_attached(&native_command(command)).map(finished)
    }

    fn is_file(&self, path: &Path) -> bool {
        OsHost.is_file(path)
    }

    fn exists(&self, path: &Path) -> bool {
        OsHost.exists(path)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        OsHost.read_file(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        OsHost.create_dir_all(path)
    }

    fn write_file(&self, path: &Path, contents: &str, private: bool) -> io::Result<()> {
        OsHost.write_file(path, contents, private)
    }

    fn say(&self, line: &str) {
        OsHost.say(line)
    }

    fn working_dir(&self) -> io::Result<std::path::PathBuf> {
        OsHost.working_dir()
    }
}

#[cfg(any(test, feature = "real"))]
fn native_command(command: &super::Command) -> NativeCommand<'_> {
    NativeCommand {
        program: command.program,
        args: &command.args,
        unset: &command.unset,
        current_dir: command.current_dir.as_deref(),
    }
}

#[cfg(any(test, feature = "real"))]
fn finished(output: NativeFinished) -> super::Finished {
    super::Finished {
        code: output.code,
        stdout: output.stdout,
        stderr: output.stderr,
    }
}
