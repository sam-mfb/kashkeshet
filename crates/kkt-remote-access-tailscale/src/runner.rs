//! The wrapper's private process boundary: tests replace it without touching a tailnet.

pub(super) trait Runner {
    fn run(&self, args: &[String]) -> Result<Output, String>;
}

pub(super) struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[cfg(feature = "real")]
pub(super) struct OsRunner;

#[cfg(feature = "real")]
impl Runner for OsRunner {
    fn run(&self, args: &[String]) -> Result<Output, String> {
        let output = std::process::Command::new("tailscale")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(launch_error)?;
        Ok(Output {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn launch_error(error: std::io::Error) -> String {
    format!(
        "could not run Tailscale: {error}; install Tailscale, start it and log in, then rerun `kkt setup`"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_launch_failure_keeps_the_install_and_login_advice() {
        assert_eq!(
            super::launch_error(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "missing executable"
            )),
            "could not run Tailscale: missing executable; install Tailscale, start it and log in, then rerun `kkt setup`"
        );
    }
}
