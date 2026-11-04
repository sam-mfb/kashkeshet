//! The real [`Host`]: runs the host's `tailscale` command.

use super::{Host, Output};
use std::process::Stdio;

pub struct OsHost;

impl Host for OsHost {
    fn run(&self, args: &[String]) -> Result<Output, String> {
        let output = std::process::Command::new("tailscale")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run Tailscale: {e}; install Tailscale, start it and log in, then rerun `kkt setup`"))?;
        Ok(Output {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
