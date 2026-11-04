//! How a client reaches the server: its address and access key file, from
//! the setup record or the serve settings, and the MCP clients made from
//! them.

use crate::machine::keys::{KeyKind, make_default_key, read_key, selected_key_path};
use crate::machine::record::read_setup;
use crate::machine::settings::{ServeArgs, load_serve_options};
use kkt_agent_client as mcp_client;
use kkt_client_config::{self as clients, Client, Setup};
use kkt_config as config;
use kkt_diagnostics as diagnostics;
use kkt_settings::{AccessKey, Env, Platform, ServeOptions};
use kkt_setup as setup;
use std::path::PathBuf;

pub struct Connection {
    pub url: String,
    pub key_file: Option<PathBuf>,
}

/// Where a client command finds the server, as given: a serve config file,
/// an address and an access key file. Each left out comes from the setup or
/// the serve settings.
pub struct ConnectionOptions {
    pub config: Option<PathBuf>,
    pub url: Option<String>,
    pub key_file: Option<PathBuf>,
}

impl ConnectionOptions {
    /// Nothing given.
    pub const NONE: ConnectionOptions = ConnectionOptions {
        config: None,
        url: None,
        key_file: None,
    };
}

/// Resolve the same serve settings without creating a database. Only connect
/// creates a missing default key; doctor is a read-only diagnostic.
pub fn client_connection(
    env: &Env,
    flags: ConnectionOptions,
    create: bool,
) -> Result<Connection, String> {
    // Without --config, clients use the server `kkt setup` or `kkt switch`
    // recorded for them; --url and --key-file each replace only their own
    // part of it.
    if flags.config.is_none()
        && let Some(record) = read_setup(env)?
    {
        return recorded_connection(record.for_clients(), flags);
    }
    serve_connection(env, flags, create)
}

/// How clients reach the recorded `server`, with `--url` and `--key-file`
/// each replacing only their own part of it.
pub fn recorded_connection(
    server: setup::Server,
    flags: ConnectionOptions,
) -> Result<Connection, String> {
    let key_file = match flags.key_file {
        Some(path) => Some(
            std::path::absolute(&path).map_err(|e| format!("could not resolve key path: {e}"))?,
        ),
        None => server.key_file,
    };
    Ok(Connection {
        url: flags.url.unwrap_or(server.url),
        key_file,
    })
}

/// How clients reach the server `kkt serve` would run with these settings.
pub fn serve_connection(
    env: &Env,
    flags: ConnectionOptions,
    create: bool,
) -> Result<Connection, String> {
    let key_override = flags.key_file.is_some();
    let loaded = load_serve_options(
        env,
        flags.config,
        ServeOptions {
            access_key_file: flags.key_file,
            no_access_key: key_override.then_some(false),
            ..ServeOptions::NONE
        },
    )?;
    let sources = loaded.sources();
    let settings = loaded
        .flags
        .over(loaded.from_file)
        .resolve(
            env.default_db(Platform::CURRENT).unwrap_or_default(),
            env.access_key_file(),
        )
        .map_err(|e| sources.invalid(e))?;
    let url = diagnostics::endpoint(settings.bind, flags.url.as_deref())?;
    if create {
        make_default_key(&settings.access_key, "access")?;
    }
    let key_file = match settings.access_key {
        AccessKey::None => None,
        AccessKey::Default(path) | AccessKey::File(path) => {
            if create {
                read_key(&path, &sources)?;
            }
            Some(
                std::path::absolute(&path)
                    .map_err(|e| format!("could not resolve key path: {e}"))?,
            )
        }
    };
    Ok(Connection { url, key_file })
}

pub fn connect(
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

/// This program's absolute path, for client configs that run it.
pub fn kkt_path() -> Result<PathBuf, String> {
    std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("could not find kkt's path: {e}"))
}

pub fn remote_client(env: &Env, flags: ConnectionOptions) -> Result<mcp_client::Client, String> {
    let connection = client_connection(env, flags, false)?;
    let key = match &connection.key_file {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|e| format!("could not read key file {}: {e}", path.display()))?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    Ok(mcp_client::Client::new(connection.url, key))
}

/// A client of the server's admin endpoint: the address `flags` and the
/// setup give, with `/mcp` changed to `/mcp-admin`, and the admin key from
/// `--key-file` or where `kkt key show --admin` finds it.
pub fn admin_client(
    env: &Env,
    serve_args: ServeArgs,
    flags: ConnectionOptions,
) -> Result<mcp_client::Client, String> {
    let key_file = flags.key_file.clone();
    let flags = ConnectionOptions {
        key_file: None,
        ..flags
    };
    // The admin key is this machine's server's, so this is that server
    // even when the clients were switched to another.
    let connection = match read_setup(env)? {
        Some(record) if flags.config.is_none() => recorded_connection(record.own_server(), flags)?,
        _ => client_connection(env, flags, false)?,
    };
    let url = match connection.url.strip_suffix("/mcp") {
        Some(base) => format!("{base}{}", config::MCP_ADMIN_PATH),
        None => {
            return Err(format!(
                "the server address {} does not end in /mcp, so the admin endpoint's cannot be worked out; pass --url ending in /mcp",
                connection.url
            ));
        }
    };
    let path = selected_key_path(env, serve_args, key_file, KeyKind::Admin)?;
    let key = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!(
            "there is no admin key at {}; `kkt key create --admin` makes one on the server",
            path.display()
        ),
        _ => format!("could not read key file {}: {e}", path.display()),
    })?;
    Ok(mcp_client::Client::new(url, Some(key.trim().to_string())))
}
