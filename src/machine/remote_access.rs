//! Adapts the machine's Tailscale command to the remote-access logic.

use kkt_remote_access::{Host, Output};
use kkt_remote_access_os::OsHost;

pub(crate) struct RemoteHost;

impl Host for RemoteHost {
    fn run(&self, args: &[String]) -> Result<Output, String> {
        let output = OsHost.run(args).map_err(|e| format!("could not run Tailscale: {e}; install Tailscale, start it and log in, then rerun `kkt setup`"))?;
        Ok(Output {
            success: output.success,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}
