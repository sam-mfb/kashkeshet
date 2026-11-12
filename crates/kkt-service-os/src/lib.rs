//! Files and commands used by the machine's service manager.
//! Plain OsSystem methods are adapted by the program to the service logic.

#[cfg(any(test, feature = "real"))]
use std::io;
#[cfg(any(test, feature = "real"))]
use std::path::Path;
#[cfg(any(test, feature = "real"))]
use std::process::Stdio;

/// The real system: files on disk and real commands.
#[cfg(any(test, feature = "real"))]
pub struct OsSystem;

#[cfg(any(test, feature = "real"))]
impl OsSystem {
    pub fn user_id(&self) -> io::Result<u32> {
        let finished = self.run("id", &["-u".into()])?;
        finished
            .stdout
            .trim()
            .parse()
            .map_err(|_| io::Error::other(format!("id -u printed {:?}", finished.stdout)))
    }

    pub fn exists(&self, path: &Path) -> io::Result<bool> {
        path.try_exists()
    }

    pub fn read_file(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    pub fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        std::fs::write(path, contents)
    }

    pub fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    pub fn run(&self, program: &str, args: &[String]) -> io::Result<Finished> {
        let output = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()?;
        Ok(Finished {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    pub fn run_attached(&self, program: &str, args: &[String]) -> io::Result<Finished> {
        let status = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .status()?;
        Ok(Finished {
            code: status.code(),
            ..Finished::default()
        })
    }
}

/// How the raw process ended, with captured output when requested.
#[derive(Default)]
pub struct Finished {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Listens on the address and lets it go at once, as the service bind check requires.
#[cfg(any(test, feature = "real"))]
pub fn try_bind(address: std::net::SocketAddr) -> io::Result<()> {
    std::net::TcpListener::bind(address).map(drop)
}
