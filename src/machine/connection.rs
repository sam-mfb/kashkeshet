//! How a client reaches the server: its address and access key file, from
//! the setup record or the serve settings. kkt-setup's connection question,
//! answered on this machine (`ConnectionReads`).

use crate::machine::keys::{make_default_key, read_key};
use crate::machine::operate::NativeSetupFiles;
use crate::machine::settings::ServeOptions;
use crate::machine::settings::Sources;
use crate::machine::settings::serve_settings_and_sources;
use kkt_admission::AccessKey;
use kkt_diagnostics as diagnostics;
use kkt_machine::{Env, Platform};
use kkt_setup as setup;
#[cfg(test)]
use setup::connection::Connection;
pub use setup::connection::ConnectionOptions;
use setup::connection::{ConnectionSource, ServeConnection};
use std::path::Path;

/// kkt-setup's connection question, answered from this machine's setup
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

    fn record(&self) -> Result<Option<setup::Record>, String> {
        kkt_operate::record::read_setup(self.env, &NativeSetupFiles)
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
    setup::connection::serve_connection(&ConnectionReads { env, platform }, flags, create)
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
