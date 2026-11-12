//! Where a client reaches the server, and with which key: the agent
//! endpoint as kkt-setup's connection question says, or the local server's
//! admin endpoint with the admin key.

use crate::{Address, AgentParts};
use kkt_setup::connection::{self, ConnectionOptions, ConnectionSource};

/// The agent endpoint, with the access key the connection names, if any.
pub(crate) fn agent<C: ConnectionSource>(
    parts: &AgentParts<C>,
    flags: ConnectionOptions,
) -> Result<Address, String> {
    let connection = connection::client_connection(&parts.connection, flags, false)?;
    let key = match &connection.key_file {
        Some(path) => Some(
            parts
                .key_files
                .read(path)
                .map_err(|e| format!("could not read key file {}: {}", path.display(), e.reason))?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    Ok(Address {
        url: connection.url,
        key,
        key_name: connection
            .key_file
            .map(|path| format!("the access key in {}", path.display())),
    })
}

/// The admin endpoint: the address `flags` and the setup give, with `/mcp`
/// changed to `/mcp-admin`, and the admin key from `--key-file` or where
/// `kkt key show --admin` finds it.
pub(crate) fn admin<C: ConnectionSource>(
    parts: &AgentParts<C>,
    flags: ConnectionOptions,
) -> Result<Address, String> {
    let key_file = flags.key_file.clone();
    let connection = connection::admin_connection(&parts.connection, flags)?;
    let url = connection::admin_endpoint(&connection.url, kkt_mcp::MCP_ADMIN_PATH)?;
    let path = parts.admin_key.path(key_file)?;
    let key = parts.key_files.read(&path).map_err(|e| match e.missing {
        true => format!(
            "there is no admin key at {}; `kkt key create --admin` makes one on the server",
            path.display()
        ),
        false => format!("could not read key file {}: {}", path.display(), e.reason),
    })?;
    Ok(Address {
        url,
        key: Some(key.trim().to_string()),
        key_name: Some(format!("the admin key in {}", path.display())),
    })
}
