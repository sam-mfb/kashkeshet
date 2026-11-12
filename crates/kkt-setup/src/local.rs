//! Setup's local paths and the advice when no record has been saved.
use kkt_machine::{Env, Platform};
use std::path::PathBuf;
const DOCKER_FOLDER: &str = "docker";

/// The MCP endpoint of kkt in Docker, published on the host's `port`, or
/// served over Tailscale as `node`, `(hostname, tailnet)`.
pub fn docker_url(port: u16, node: Option<&(String, String)>) -> String {
    match node {
        Some((hostname, tailnet)) => format!("https://{hostname}.{tailnet}/mcp"),
        None => format!("http://127.0.0.1:{port}/mcp"),
    }
}

/// The mode of the server `unfinished` notes: from its record, or, for an
/// install that did not finish, where that install puts the server.
pub fn unfinished_mode(
    env: &Env,
    platform: Platform,
    unfinished: &crate::Unfinished,
) -> Result<crate::Mode, String> {
    unfinished.mode(docker_folder(env, platform))
}

/// The folder `kkt setup` keeps kkt's Docker files in.
pub fn docker_folder(env: &Env, platform: Platform) -> Result<PathBuf, String> {
    Ok(crate::data_dir(env, platform)
        .ok_or("set HOME to keep kkt's Docker files")?
        .join(DOCKER_FOLDER))
}

/// This machine's setup record. Without one, the error says so, and names a
/// server kkt installed here anyway, which `kkt setup` can record without
/// installing it again.
pub fn recorded_setup(
    record: Option<crate::Record>,
    unfinished: impl FnOnce() -> Result<Option<crate::Unfinished>, String>,
    unrecorded: impl FnOnce() -> Result<Option<crate::Unrecorded>, (crate::Purpose, String)>,
) -> Result<crate::Record, String> {
    if let Some(record) = record {
        return Ok(record);
    }
    let no_record = "this machine has no setup record";
    // An unfinished setup is described by `kkt status`.
    match unfinished() {
        Ok(Some(_)) => return Err(format!("{no_record}; run `kkt setup` to finish it")),
        Ok(None) => {}
        Err(e) => return Err(format!("{no_record}; run `kkt setup` first ({e})")),
    }
    Err(match unrecorded() {
        Ok(Some(found)) => format!(
            "{no_record}. {} Run `kkt setup` to record it (`kkt setup --adopt` with no \
             terminal); it installs nothing. Then run this again.",
            found.summary()
        ),
        Ok(None) => format!("{no_record}; run `kkt setup` first"),
        Err((_, e)) => format!("{no_record}; run `kkt setup` first. {e}"),
    })
}
