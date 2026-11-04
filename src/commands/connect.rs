//! `kkt connect`: writes or prints a client's MCP config for this machine's
//! server.

use crate::machine::connection::{ConnectionOptions, client_connection};
use crate::process::kkt_path;
use kkt_client_config::{self as clients, Client, Setup};
use kkt_settings::Env;
use std::path::PathBuf;

pub fn run(
    env: &Env,
    client: &str,
    flags: ConnectionOptions,
    client_config: Option<PathBuf>,
    write: bool,
) -> Result<(), String> {
    let client = Client::from_name(client).ok_or("unknown client")?;
    let (client_config, source) = match client_config {
        Some(path) => (path, "--client-config"),
        None => (client.file(env)?, client.source(env)),
    };
    // Refuse a wrong name before the connection creates a missing key.
    client.check_name(&client_config, source)?;
    let connection = client_connection(env, flags, true)?;
    let executable = kkt_path()?;
    let setup = Setup {
        env,
        url: &connection.url,
        executable: &executable,
        key_file: connection.key_file.as_deref(),
        client_config: &client_config,
        source,
    };
    println!("{}", clients::configure(client, &setup, write)?);
    Ok(())
}
