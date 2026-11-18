//! The wrapper's private process boundary, which tests replace.

pub(super) trait Runner {
    fn run(&self, args: &[String]) -> Result<Output, RunError>;
}

/// Why `tailscale` could not be run: `missing` when there is no such
/// program, and what to tell the person.
#[derive(Debug)]
pub(super) struct RunError {
    pub missing: bool,
    pub said: String,
}

impl From<RunError> for String {
    fn from(error: RunError) -> String {
        error.said
    }
}

pub(super) struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs the machine's `tailscale`. `rerun` is the command a person should
/// rerun once Tailscale runs, which a launch failure names.
pub(super) struct OsRunner {
    pub rerun: &'static str,
}

impl Runner for OsRunner {
    fn run(&self, args: &[String]) -> Result<Output, RunError> {
        let output = std::process::Command::new("tailscale")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| RunError {
                missing: e.kind() == std::io::ErrorKind::NotFound,
                said: launch_error(e, self.rerun),
            })?;
        Ok(Output {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn launch_error(error: std::io::Error, rerun: &str) -> String {
    format!(
        "could not run Tailscale: {error}; install Tailscale, start it and log in, then rerun {rerun}"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_launch_failure_keeps_the_install_and_login_advice() {
        assert_eq!(
            super::launch_error(
                std::io::Error::new(std::io::ErrorKind::NotFound, "missing executable"),
                "`kkt setup`"
            ),
            "could not run Tailscale: missing executable; install Tailscale, start it and log in, then rerun `kkt setup`"
        );
    }
}
