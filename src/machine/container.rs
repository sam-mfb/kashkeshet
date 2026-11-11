//! Adapts the machine's Docker operations to the container logic's host.

use kkt_container::{Command, Finished, Host};
use kkt_container_os::{self as native, OsHost};
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct ContainerHost;

impl Host for ContainerHost {
    fn run(&self, command: &Command) -> io::Result<Finished> {
        OsHost.run(&native_command(command)).map(finished)
    }

    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        OsHost.run_attached(&native_command(command)).map(finished)
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

    fn working_dir(&self) -> io::Result<PathBuf> {
        OsHost.working_dir()
    }
}

fn native_command(command: &Command) -> native::Command<'_> {
    native::Command {
        program: command.program,
        args: &command.args,
        unset: &command.unset,
        current_dir: command.current_dir.as_deref(),
    }
}

fn finished(output: native::Finished) -> Finished {
    Finished {
        code: output.code,
        stdout: output.stdout,
        stderr: output.stderr,
    }
}
