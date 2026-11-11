//! The machine's Tailscale command. The program adapts this raw runner to
//! the remote-access logic's host contract.

/// The process's success status and captured output.
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The machine's `tailscale` command.
#[cfg(feature = "real")]
pub struct OsHost;

#[cfg(feature = "real")]
impl OsHost {
    pub fn run(&self, args: &[String]) -> std::io::Result<Output> {
        let output = std::process::Command::new("tailscale")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()?;
        Ok(Output {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
