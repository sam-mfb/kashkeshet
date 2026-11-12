//! The ordered choice of a client's endpoint and key, from setup or serve settings.

use crate::Record;
use kkt_admission::AccessKey;
use std::path::{Path, PathBuf};

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

/// Reads needed by connection selection, requested only in their original order.
/// The program supplies settings and adapts its native providers.
pub trait ConnectionSource {
    /// Context used to describe a key-setting refusal, opaque to setup.
    type KeySource;
    fn record(&self) -> Result<Option<Record>, String>;
    fn serve(&self, flags: ConnectionOptions) -> Result<ServeConnection<Self::KeySource>, String>;
    fn make_default_key(&self, key: &AccessKey) -> Result<(), String>;
    fn check_key(&self, path: &Path, source: &Self::KeySource) -> Result<(), String>;
    fn absolute(&self, path: &Path) -> Result<PathBuf, String>;
}

/// The checked endpoint and access-key setting; source context is returned to its reader.
pub struct ServeConnection<S> {
    pub url: String,
    pub access_key: AccessKey,
    pub key_source: S,
}

/// Resolve the same serve settings without creating a database. Only connect
/// creates a missing default key; doctor is a read-only diagnostic.
pub fn client_connection(
    source: &impl ConnectionSource,
    flags: ConnectionOptions,
    create: bool,
) -> Result<Connection, String> {
    // Without --config, clients use the server `kkt setup` or `kkt switch`
    // recorded for them; --url and --key-file each replace only their own
    // part of it.
    if flags.config.is_none()
        && let Some(record) = source.record()?
    {
        return recorded_connection(|path| source.absolute(path), record.for_clients(), flags);
    }
    serve_connection(source, flags, create)
}

/// The local server's connection for admin tools, even when clients were switched.
/// The first record read remains unconditional, including when --config is given.
pub fn admin_connection(
    source: &impl ConnectionSource,
    flags: ConnectionOptions,
) -> Result<Connection, String> {
    // --key-file names the admin key; it must not replace the access key used
    // to select this machine's server.
    let flags = ConnectionOptions {
        key_file: None,
        ..flags
    };
    match source.record()? {
        Some(record) if flags.config.is_none() => {
            recorded_connection(|path| source.absolute(path), record.own_server(), flags)
        }
        _ => client_connection(source, flags, false),
    }
}

/// The admin endpoint, from a client endpoint ending in /mcp.
/// The program supplies the transport's admin path constant.
pub fn admin_endpoint(url: &str, admin_path: &str) -> Result<String, String> {
    match url.strip_suffix("/mcp") {
        Some(base) => Ok(format!("{base}{admin_path}")),
        None => Err(format!(
            "the server address {url} does not end in /mcp, so the admin endpoint's cannot be worked out; pass --url ending in /mcp"
        )),
    }
}

/// How clients reach the recorded `server`, with `--url` and `--key-file`
/// each replacing only their own part of it.
pub fn recorded_connection(
    absolute: impl FnOnce(&Path) -> Result<PathBuf, String>,
    server: crate::Server,
    flags: ConnectionOptions,
) -> Result<Connection, String> {
    let key_file = match flags.key_file {
        Some(path) => Some(absolute(&path)?),
        None => server.key_file,
    };
    Ok(Connection {
        url: flags.url.unwrap_or(server.url),
        key_file,
    })
}

/// How clients reach the server `kkt serve` would run with these settings.
pub fn serve_connection(
    source: &impl ConnectionSource,
    flags: ConnectionOptions,
    create: bool,
) -> Result<Connection, String> {
    let ServeConnection {
        url,
        access_key,
        key_source,
    } = source.serve(flags)?;
    if create {
        source.make_default_key(&access_key)?;
    }
    let key_file = match access_key {
        AccessKey::None => None,
        AccessKey::Default(path) | AccessKey::File(path) => {
            if create {
                source.check_key(&path, &key_source)?;
            }
            Some(source.absolute(&path)?)
        }
    };
    Ok(Connection { url, key_file })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Reads {
        calls: RefCell<Vec<&'static str>>,
        bad_record: bool,
    }

    impl ConnectionSource for Reads {
        type KeySource = ();
        fn record(&self) -> Result<Option<Record>, String> {
            self.calls.borrow_mut().push("record");
            if self.bad_record {
                Err("bad record".into())
            } else {
                Ok(None)
            }
        }
        fn serve(&self, _: ConnectionOptions) -> Result<ServeConnection<()>, String> {
            self.calls.borrow_mut().push("serve");
            Ok(ServeConnection {
                url: "http://server/mcp".into(),
                access_key: AccessKey::File("key".into()),
                key_source: (),
            })
        }
        fn make_default_key(&self, _: &AccessKey) -> Result<(), String> {
            self.calls.borrow_mut().push("make key");
            Ok(())
        }
        fn check_key(&self, _: &Path, _: &()) -> Result<(), String> {
            self.calls.borrow_mut().push("check key");
            Err("bad key".into())
        }
        fn absolute(&self, path: &Path) -> Result<PathBuf, String> {
            self.calls.borrow_mut().push("absolute");
            Ok(path.to_owned())
        }
    }

    #[test]
    fn record_refusal_precedes_override_resolution_and_config_skips_the_record() {
        let source = Reads {
            calls: RefCell::new(vec![]),
            bad_record: true,
        };
        let flags = ConnectionOptions {
            key_file: Some("override".into()),
            ..ConnectionOptions::NONE
        };
        assert_eq!(
            client_connection(&source, flags, true).map(|c| c.url),
            Err("bad record".into())
        );
        assert_eq!(*source.calls.borrow(), ["record"]);

        source.calls.borrow_mut().clear();
        let flags = ConnectionOptions {
            config: Some("config".into()),
            ..ConnectionOptions::NONE
        };
        assert_eq!(
            admin_connection(&source, flags).map(|c| c.url),
            Err("bad record".into())
        );
        assert_eq!(*source.calls.borrow(), ["record"]);

        source.calls.borrow_mut().clear();
        let flags = ConnectionOptions {
            config: Some("config".into()),
            ..ConnectionOptions::NONE
        };
        assert_eq!(
            client_connection(&source, flags, true).map(|c| c.url),
            Err("bad key".into())
        );
        assert_eq!(*source.calls.borrow(), ["serve", "make key", "check key"]);
    }

    #[test]
    fn read_only_selection_resolves_without_making_or_reading_a_key() {
        let source = Reads {
            calls: RefCell::new(vec![]),
            bad_record: false,
        };
        let connection =
            client_connection(&source, ConnectionOptions::NONE, false).expect("connection");
        assert_eq!(connection.key_file, Some("key".into()));
        assert_eq!(*source.calls.borrow(), ["record", "serve", "absolute"]);
    }
}
