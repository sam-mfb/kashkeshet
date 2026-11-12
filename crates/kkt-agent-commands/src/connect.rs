//! `kkt connect`: writes or prints a client's MCP config for this machine's
//! server.

use crate::AgentParts;
use kkt_client_config::{self as clients, Client, Setup};
use kkt_machine::Env;
use kkt_setup::connection::{self, ConnectionOptions, ConnectionSource};
use std::path::PathBuf;

/// Writes the config of the client named `client`, at `client_config` or
/// where that client keeps it, or only prints it unless `write`. A missing
/// default access key is made.
pub fn run<C: ConnectionSource>(
    parts: &AgentParts<C>,
    env: &Env,
    client: &str,
    flags: ConnectionOptions,
    client_config: Option<PathBuf>,
    write: bool,
) -> Result<(), String> {
    let client = Client::from_name(client).ok_or("unknown client")?;
    let (client_config, source) = match client_config {
        Some(path) => (path, "--client-config"),
        None => (client.file(env, &*parts.clients)?, client.source(env)),
    };
    // Refuse a wrong name before the connection creates a missing key.
    client.check_name(&client_config, source)?;
    let connection = connection::client_connection(&parts.connection, flags, true)?;
    let executable = parts.program.kkt_path()?;
    let setup = Setup {
        env,
        url: &connection.url,
        executable: &executable,
        key_file: connection.key_file.as_deref(),
        client_config: &client_config,
        source,
    };
    parts.output.output(&format!(
        "{}\n",
        clients::configure(
            client,
            &setup,
            write,
            &*parts.clients,
            &*parts.client_command
        )?
    ));
    Ok(())
}
