//! How a client reaches the server: its address and access key file, from
//! the setup record or the serve settings. kkt-deployment's connection question,
//! answered on this machine (`ConnectionReads`).

use crate::machine::keys::{make_default_key, read_key};
use crate::machine::native::{NativeSetupFiles, ServeReads};
use crate::machine::settings::ServeOptions;
use crate::machine::settings::Sources;
use crate::machine::settings::serve_settings_and_sources;
/// The MCP endpoint a client of the serve settings reaches: `given`, as
/// kkt-deployment's address rule takes it, or with none given the server on
/// `bind`. A bare address is given `mcp_path`.
pub fn serve_endpoint(
    bind: std::net::SocketAddr,
    given: Option<&str>,
    mcp_path: &str,
) -> Result<String, String> {
    let default = format!("http://{bind}{mcp_path}");
    kkt_deployment::connection::server_address(given.unwrap_or(&default), Some(mcp_path))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
use kkt_deployment::connection::Connection;
pub use kkt_deployment::connection::ConnectionOptions;
use kkt_deployment::connection::{ConnectionSource, RecordedServer, ServeConnection, SourceError};
use kkt_keys::AccessKey;
use kkt_machine::{Env, Platform};
use std::path::Path;

/// kkt-deployment's connection question, answered from this machine's setup
/// record and serve settings.
pub struct ConnectionReads<'a> {
    pub(crate) env: &'a Env,
    pub(crate) platform: Platform,
}

impl<'a> ConnectionReads<'a> {
    pub fn new(env: &'a Env, platform: Platform) -> Self {
        Self { env, platform }
    }
}

impl ConnectionSource for ConnectionReads<'_> {
    type KeySource = Sources;

    fn recorded(&self) -> Result<Option<RecordedServer>, SourceError> {
        Ok(kkt_install_commands::record::read_setup(
            &ServeReads::new(self.env, self.platform),
            &NativeSetupFiles,
        )
        .map_err(source_error)?
        .map(|record| kkt_install_commands::deployment::recorded_server(&record)))
    }

    fn serve(&self, flags: ConnectionOptions) -> Result<ServeConnection<Sources>, SourceError> {
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
        )
        .map_err(source_error)?;
        let url = serve_endpoint(settings.bind, flags.url.as_deref(), kkt_mcp::MCP_PATH)
            .map_err(source_error)?;
        Ok(ServeConnection {
            url,
            access_key: settings.access_key,
            key_source: sources,
        })
    }

    fn make_default_key(&self, key: &AccessKey) -> Result<(), SourceError> {
        make_default_key(key, "access").map_err(source_error)
    }

    fn check_key(&self, path: &Path, source: &Sources) -> Result<(), SourceError> {
        read_key(path, source).map(|_| ()).map_err(source_error)
    }

    fn absolute(&self, path: &Path) -> Result<std::path::PathBuf, SourceError> {
        kkt_files::Files
            .absolute(path)
            .map_err(|e| source_error(format!("could not resolve key path: {e}")))
    }
}

/// This program's reads still say why in words: kkt-deployment's contract
/// carries them as they are.
fn source_error(said: String) -> SourceError {
    SourceError(said.into())
}

/// How clients reach the server `kkt serve` would run with these settings,
/// as kkt-setup works it out over this machine's reads: for the tests of
/// those reads.
#[cfg(test)]
fn serve_connection(
    env: &Env,
    platform: Platform,
    flags: ConnectionOptions,
    create: bool,
) -> Result<Connection, String> {
    kkt_deployment::connection::serve_connection(&ConnectionReads { env, platform }, flags, create)
        .map_err(|e| e.to_string())
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
