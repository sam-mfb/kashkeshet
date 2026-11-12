//! How a client reaches the server: its address and access key file, from
//! the setup record or the serve settings, and the MCP clients made from
//! them.

use crate::machine::keys::{KeyKind, make_default_key, read_key, selected_key_path};
use crate::machine::record::read_setup;
use crate::machine::settings::ServeOptions;
use crate::machine::settings::Sources;
use crate::machine::settings::{ServeArgs, serve_settings_and_sources};
use kkt_admission::AccessKey;
use kkt_diagnostics as diagnostics;
use kkt_machine::{Env, Platform};
use kkt_setup as setup;
pub use setup::connection::{Connection, ConnectionOptions};
use setup::connection::{ConnectionSource, ServeConnection};
use std::path::Path;

struct ConnectionReads<'a> {
    env: &'a Env,
    platform: Platform,
}

impl ConnectionSource for ConnectionReads<'_> {
    type KeySource = Sources;

    fn record(&self) -> Result<Option<setup::Record>, String> {
        read_setup(self.env)
    }

    fn serve(&self, flags: ConnectionOptions) -> Result<ServeConnection<Sources>, String> {
        let key_override = flags.key_file.is_some();
        let (settings, sources) = serve_settings_and_sources(
            self.env,
            self.platform,
            flags.config,
            ServeOptions {
                access_key_file: flags.key_file,
                no_access_key: key_override.then_some(false),
                ..ServeOptions::NONE
            },
        )?;
        let url = diagnostics::endpoint(settings.bind, flags.url.as_deref())?;
        Ok(ServeConnection {
            url,
            access_key: settings.access_key,
            key_source: sources,
        })
    }

    fn make_default_key(&self, key: &AccessKey) -> Result<(), String> {
        make_default_key(key, "access")
    }

    fn check_key(&self, path: &Path, source: &Sources) -> Result<(), String> {
        read_key(path, source).map(|_| ())
    }

    fn absolute(&self, path: &Path) -> Result<std::path::PathBuf, String> {
        kkt_setup_os::absolute(path).map_err(|e| format!("could not resolve key path: {e}"))
    }
}

pub fn client_connection(
    env: &Env,
    platform: Platform,
    flags: ConnectionOptions,
    create: bool,
) -> Result<Connection, String> {
    setup::connection::client_connection(&ConnectionReads { env, platform }, flags, create)
}

pub fn recorded_connection(
    server: setup::Server,
    flags: ConnectionOptions,
) -> Result<Connection, String> {
    setup::connection::recorded_connection(
        |path| kkt_setup_os::absolute(path).map_err(|e| format!("could not resolve key path: {e}")),
        server,
        flags,
    )
}

pub fn serve_connection(
    env: &Env,
    platform: Platform,
    flags: ConnectionOptions,
    create: bool,
) -> Result<Connection, String> {
    setup::connection::serve_connection(&ConnectionReads { env, platform }, flags, create)
}

pub(crate) fn remote_client(
    env: &Env,
    platform: Platform,
    flags: ConnectionOptions,
) -> Result<kkt_agent_client::Client, String> {
    let connection = client_connection(env, platform, flags, false)?;
    let key = match &connection.key_file {
        Some(path) => Some(
            kkt_admission_os::KeyFiles::read(path)
                .map_err(|e| format!("could not read key file {}: {}", path.display(), e.error))?
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
    platform: Platform,
    serve_args: ServeArgs,
    flags: ConnectionOptions,
) -> Result<kkt_agent_client::Client, String> {
    let key_file = flags.key_file.clone();
    let connection =
        setup::connection::admin_connection(&ConnectionReads { env, platform }, flags)?;
    let url = setup::connection::admin_endpoint(&connection.url, kkt_mcp::MCP_ADMIN_PATH)?;
    let path = selected_key_path(env, platform, serve_args, key_file, KeyKind::Admin)?;
    let key = kkt_admission_os::KeyFiles::read(&path).map_err(|e| match e.error.kind() {
        std::io::ErrorKind::NotFound => format!(
            "there is no admin key at {}; `kkt key create --admin` makes one on the server",
            path.display()
        ),
        _ => format!("could not read key file {}: {}", path.display(), e.error),
    })?;
    Ok(
        kkt_agent_client::Client::new(url, Some(key.trim().to_string()))
            .describe_key(format!("the admin key in {}", path.display())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::settings::{TEST_PLATFORM, test_env, write_default_config};
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
        let connection =
            serve_connection(&test_env(None), TEST_PLATFORM, flags, false).expect("valid");
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
        let connection =
            serve_connection(&test_env(None), TEST_PLATFORM, flags, false).expect("valid");
        assert_eq!(connection.url, "http://127.0.0.1:7420/mcp");
        assert_eq!(connection.key_file, Some(key));
    }

    #[test]
    fn without_a_home_or_a_key_the_connection_is_refused() {
        assert_eq!(
            serve_connection(
                &test_env(None),
                TEST_PLATFORM,
                ConnectionOptions::NONE,
                false
            )
            .map(|c| c.url),
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
        let connection =
            serve_connection(&env, TEST_PLATFORM, ConnectionOptions::NONE, false).expect("valid");
        assert_eq!(
            connection.key_file,
            crate::machine::settings::default_access_key(&env)
        );
        let path = write_default_config(&home, "name_pack = \"missing\"");
        let err = serve_connection(&env, TEST_PLATFORM, ConnectionOptions::NONE, false)
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
