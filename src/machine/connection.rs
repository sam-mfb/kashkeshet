//! How a client reaches the server: its address and access key file, from
//! the setup record or the serve settings, and the MCP clients made from
//! them.

use crate::machine::keys::{KeyKind, make_default_key, read_key, selected_key_path};
use crate::machine::record::read_setup;
use crate::machine::settings::{ServeArgs, serve_settings_and_sources};
use kkt_config as config;
use kkt_diagnostics as diagnostics;
use kkt_settings::{AccessKey, Env, ServeOptions};
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
    let (settings, sources) = serve_settings_and_sources(
        env,
        flags.config,
        ServeOptions {
            access_key_file: flags.key_file,
            no_access_key: key_override.then_some(false),
            ..ServeOptions::NONE
        },
    )?;
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

pub(crate) fn remote_client(
    env: &Env,
    flags: ConnectionOptions,
) -> Result<kkt_agent_client::Client, String> {
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
    let client = kkt_agent_client::Client::new(connection.url, key);
    Ok(match &connection.key_file {
        Some(path) => client.describe_key(format!("the access key in {}", path.display())),
        None => client,
    })
}

/// A client of the server's admin endpoint: the address `flags` and the
/// setup give, with `/mcp` changed to `/mcp-admin`, and the admin key from
/// `--key-file` or where `kkt key show --admin` finds it.
pub(crate) fn admin_client(
    env: &Env,
    serve_args: ServeArgs,
    flags: ConnectionOptions,
) -> Result<kkt_agent_client::Client, String> {
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
    Ok(
        kkt_agent_client::Client::new(url, Some(key.trim().to_string()))
            .describe_key(format!("the admin key in {}", path.display())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{test_env, write_default_config};
    use crate::scratch;

    #[test]
    fn without_a_home_a_config_without_a_key_still_gives_the_address() {
        let dir = scratch::Dir::new("connection-no-home");
        let config = dir.join("serve.toml");
        std::fs::write(&config, "no_access_key = true\nbind = \"127.0.0.1:7555\"").expect("file");
        let flags = ConnectionOptions {
            config: Some(config),
            ..ConnectionOptions::NONE
        };
        let connection = serve_connection(&test_env(None), flags, false).expect("valid");
        assert_eq!(connection.url, "http://127.0.0.1:7555/mcp");
        assert_eq!(connection.key_file, None);
    }

    #[test]
    fn without_a_home_a_given_key_file_needs_no_default() {
        let dir = scratch::Dir::new("connection-no-home-key");
        let key = dir.join("key");
        let flags = ConnectionOptions {
            key_file: Some(key.clone()),
            ..ConnectionOptions::NONE
        };
        let connection = serve_connection(&test_env(None), flags, false).expect("valid");
        assert_eq!(connection.url, "http://127.0.0.1:7420/mcp");
        assert_eq!(connection.key_file, Some(key));
    }

    #[test]
    fn without_a_home_or_a_key_the_connection_is_refused() {
        assert_eq!(
            serve_connection(&test_env(None), ConnectionOptions::NONE, false).map(|c| c.url),
            Err(
                "invalid setting: access_key_file has no default without a home directory: \
                 set HOME, or give access_key_file or no_access_key"
                    .to_string()
            )
        );
    }

    #[test]
    fn with_a_home_the_default_key_and_the_file_setting_are_used() {
        let home = scratch::Dir::new("connection-home");
        let env = test_env(Some(&home));
        let connection = serve_connection(&env, ConnectionOptions::NONE, false).expect("valid");
        assert_eq!(connection.key_file, env.access_key_file());
        let path = write_default_config(&home, "name_pack = \"missing\"");
        let err = serve_connection(&env, ConnectionOptions::NONE, false)
            .map(|c| c.url)
            .expect_err("bad pack");
        assert!(
            err.starts_with("invalid setting: name_pack must be one of: "),
            "{err}"
        );
        assert!(
            err.ends_with(&format!(" (set in {})", path.display())),
            "{err}"
        );
    }
}
