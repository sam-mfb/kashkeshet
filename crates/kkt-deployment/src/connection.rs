//! How a client reaches the server: its endpoint and access key file, from
//! the servers this machine's record names, or else from the serve settings.

use kkt_keys::AccessKey;
use std::path::{Path, PathBuf};
use url::Url;

/// A server, as this machine's clients reach it: its MCP endpoint, and the
/// file holding its access key, if it has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAddress {
    pub url: String,
    pub key_file: Option<PathBuf>,
}

/// The servers this machine's setup record names: the one set up here (on
/// a client machine, the one it uses), and the other server its clients
/// were switched to, if they were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedServer {
    pub server: ServerAddress,
    pub switched_to: Option<ServerAddress>,
}

impl RecordedServer {
    /// The server set up here, or on a client machine the one it uses.
    pub fn own_server(&self) -> ServerAddress {
        self.server.clone()
    }

    /// The server this machine's clients use: the one they were switched
    /// to, if they were, else [`RecordedServer::own_server`].
    pub fn for_clients(&self) -> ServerAddress {
        self.switched_to
            .clone()
            .unwrap_or_else(|| self.own_server())
    }
}

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
    /// The servers this machine's setup record names, if it has one.
    fn recorded(&self) -> Result<Option<RecordedServer>, String>;
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
        && let Some(record) = source.recorded()?
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
    match source.recorded()? {
        Some(record) if flags.config.is_none() => {
            recorded_connection(|path| source.absolute(path), record.own_server(), flags)
        }
        _ => client_connection(source, flags, false),
    }
}

/// The admin endpoint, from a client endpoint ending in `mcp_path`. The
/// program supplies the transport's two path constants.
pub fn admin_endpoint(url: &str, mcp_path: &str, admin_path: &str) -> Result<String, String> {
    match url.strip_suffix(mcp_path) {
        Some(base) => Ok(format!("{base}{admin_path}")),
        None => Err(format!(
            "the server address {url} does not end in {mcp_path}, so the admin endpoint's cannot be worked out; pass --url ending in {mcp_path}"
        )),
    }
}

/// How clients reach the recorded `server`, with `--url` and `--key-file`
/// each replacing only their own part of it.
pub fn recorded_connection(
    absolute: impl FnOnce(&Path) -> Result<PathBuf, String>,
    server: ServerAddress,
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

/// Why an address is not a server address a client may be given.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    /// It holds a control character, such as a line break.
    #[error("URL contains a control character")]
    ControlCharacter,
    /// It is not an absolute http:// or https:// address with a host.
    #[error("URL must be an absolute http:// or https:// address")]
    NotAbsolute,
    /// It holds a user name or a password, which belong in the key file.
    #[error(
        "put credentials in the key file, not in the URL; query strings and fragments are not supported"
    )]
    Credentials,
    /// It holds a query string.
    #[error(
        "put credentials in the key file, not in the URL; query strings and fragments are not supported"
    )]
    Query,
    /// It holds a fragment.
    #[error(
        "put credentials in the key file, not in the URL; query strings and fragments are not supported"
    )]
    Fragment,
}

/// `given` as a server address a client may be given: an absolute http://
/// or https:// address with a host, and no control character, user name,
/// password, query or fragment, so no secret is kept in it. With
/// `bare_path`, an address with no path is given that path; a path
/// supplied is left as it is, for a server behind a forwarding setup.
pub fn server_address(given: &str, bare_path: Option<&str>) -> Result<String, AddressError> {
    if given.chars().any(char::is_control) {
        return Err(AddressError::ControlCharacter);
    }
    let mut url = Url::parse(given).map_err(|_| AddressError::NotAbsolute)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AddressError::NotAbsolute);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AddressError::Credentials);
    }
    if url.query().is_some() {
        return Err(AddressError::Query);
    }
    if url.fragment().is_some() {
        return Err(AddressError::Fragment);
    }
    if let Some(path) = bare_path
        && url.path() == "/"
    {
        url.set_path(path);
    }
    Ok(url.to_string())
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
        fn recorded(&self) -> Result<Option<RecordedServer>, String> {
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

    /// A source whose record names its own server and, when switched, the
    /// server its clients use; the serve settings are never asked.
    struct Recorded(Option<RecordedServer>);

    impl ConnectionSource for Recorded {
        type KeySource = ();
        fn recorded(&self) -> Result<Option<RecordedServer>, String> {
            Ok(self.0.clone())
        }
        fn serve(&self, _: ConnectionOptions) -> Result<ServeConnection<()>, String> {
            panic!("the serve settings were asked while a record was there")
        }
        fn make_default_key(&self, _: &AccessKey) -> Result<(), String> {
            panic!("a key was made while a record was there")
        }
        fn check_key(&self, _: &Path, _: &()) -> Result<(), String> {
            panic!("a key was checked while a record was there")
        }
        fn absolute(&self, path: &Path) -> Result<PathBuf, String> {
            Ok(Path::new("/here").join(path))
        }
    }

    fn record(clients: Option<ServerAddress>) -> RecordedServer {
        RecordedServer {
            server: ServerAddress {
                url: "http://127.0.0.1:7420/mcp".into(),
                key_file: Some("/home/me/.config/kkt/access-key".into()),
            },
            switched_to: clients,
        }
    }

    #[test]
    fn clients_use_the_switched_server_and_admin_tools_this_machines() {
        let switched = ServerAddress {
            url: "https://other.tail1234.ts.net/mcp".into(),
            key_file: Some("/home/me/.config/kkt/other-key".into()),
        };
        let source = Recorded(Some(record(Some(switched))));
        let clients = client_connection(&source, ConnectionOptions::NONE, true).expect("clients");
        assert_eq!(clients.url, "https://other.tail1234.ts.net/mcp");
        assert_eq!(
            clients.key_file,
            Some("/home/me/.config/kkt/other-key".into())
        );
        let admin = admin_connection(&source, ConnectionOptions::NONE).expect("admin");
        assert_eq!(admin.url, "http://127.0.0.1:7420/mcp");
        assert_eq!(
            admin.key_file,
            Some("/home/me/.config/kkt/access-key".into())
        );

        let unswitched = Recorded(Some(record(None)));
        let clients =
            client_connection(&unswitched, ConnectionOptions::NONE, true).expect("clients");
        assert_eq!(clients.url, "http://127.0.0.1:7420/mcp");
        assert_eq!(
            clients.key_file,
            Some("/home/me/.config/kkt/access-key".into())
        );
    }

    #[test]
    fn url_and_key_file_each_replace_only_their_own_part_of_the_record() {
        let source = Recorded(Some(record(None)));
        let url_only = ConnectionOptions {
            url: Some("http://elsewhere:9000/mcp".into()),
            ..ConnectionOptions::NONE
        };
        let connection = client_connection(&source, url_only, true).expect("url");
        assert_eq!(connection.url, "http://elsewhere:9000/mcp");
        assert_eq!(
            connection.key_file,
            Some("/home/me/.config/kkt/access-key".into())
        );
        let key_only = ConnectionOptions {
            key_file: Some("mine".into()),
            ..ConnectionOptions::NONE
        };
        let connection = client_connection(&source, key_only, true).expect("key");
        assert_eq!(connection.url, "http://127.0.0.1:7420/mcp");
        assert_eq!(connection.key_file, Some("/here/mine".into()));
        // The admin tools' --key-file names the admin key, so it never
        // replaces the access key that picks this machine's server.
        let key_only = ConnectionOptions {
            key_file: Some("mine".into()),
            ..ConnectionOptions::NONE
        };
        let admin = admin_connection(&source, key_only).expect("admin");
        assert_eq!(
            admin.key_file,
            Some("/home/me/.config/kkt/access-key".into())
        );
    }

    #[test]
    fn the_admin_endpoint_replaces_the_mcp_path() {
        assert_eq!(
            admin_endpoint("https://kkt.tail1234.ts.net/mcp", "/mcp", "/mcp-admin"),
            Ok("https://kkt.tail1234.ts.net/mcp-admin".into())
        );
        assert_eq!(
            admin_endpoint("http://127.0.0.1:7420/other", "/mcp", "/mcp-admin"),
            Err("the server address http://127.0.0.1:7420/other does not end in /mcp, so the admin endpoint's cannot be worked out; pass --url ending in /mcp".into())
        );
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

    #[test]
    fn an_address_without_a_path_is_given_the_mcp_path_and_secrets_are_refused() {
        assert_eq!(
            server_address("http://127.0.0.1:7420/mcp", Some("/mcp")),
            Ok("http://127.0.0.1:7420/mcp".to_string())
        );
        assert_eq!(
            server_address("https://kkt.tail1234.ts.net", Some("/mcp")).map_err(|e| e.to_string()),
            Ok("https://kkt.tail1234.ts.net/mcp".to_string())
        );
        assert_eq!(
            server_address("https://kkt.tail1234.ts.net/", Some("/mcp")).map_err(|e| e.to_string()),
            Ok("https://kkt.tail1234.ts.net/mcp".to_string())
        );
        assert_eq!(
            server_address("http://h/prefix/mcp", Some("/mcp")).map_err(|e| e.to_string()),
            Ok("http://h/prefix/mcp".to_string())
        );
        let absolute = "URL must be an absolute http:// or https:// address".to_string();
        assert_eq!(
            server_address("ftp://h/mcp", Some("/mcp")).map_err(|e| e.to_string()),
            Err(absolute.clone())
        );
        assert_eq!(
            server_address("not a url", Some("/mcp")).map_err(|e| e.to_string()),
            Err(absolute)
        );
        assert_eq!(
            server_address("http://h/\u{7}", Some("/mcp")).map_err(|e| e.to_string()),
            Err("URL contains a control character".to_string())
        );
        let secret = "put credentials in the key file, not in the URL; query strings and fragments are not supported".to_string();
        for refused in [
            "http://u:p@h/mcp",
            "http://u@h/mcp",
            "http://:p@h/mcp",
            "http://h/mcp?q=1",
            "http://h/mcp#f",
        ] {
            assert_eq!(
                server_address(refused, Some("/mcp")).map_err(|e| e.to_string()),
                Err(secret.clone()),
                "{refused}"
            );
        }
        assert_eq!(
            server_address("file:///tmp/x", Some("/mcp")).map_err(|e| e.to_string()),
            Err("URL must be an absolute http:// or https:// address".to_string())
        );
    }

    /// An address already in the setup record is used as it is: the address
    /// rule is applied when an address is given, never to one recorded.
    #[test]
    fn a_recorded_address_is_not_checked_again() {
        struct Recorded;
        impl ConnectionSource for Recorded {
            type KeySource = ();
            fn recorded(&self) -> Result<Option<RecordedServer>, String> {
                Ok(Some(RecordedServer {
                    server: ServerAddress {
                        url: "http://h/mcp?q=1".into(),
                        key_file: None,
                    },
                    switched_to: None,
                }))
            }
            fn serve(&self, _: ConnectionOptions) -> Result<ServeConnection<()>, String> {
                panic!("a recorded server is not read from the serve settings")
            }
            fn make_default_key(&self, _: &AccessKey) -> Result<(), String> {
                Ok(())
            }
            fn check_key(&self, _: &Path, _: &()) -> Result<(), String> {
                Ok(())
            }
            fn absolute(&self, path: &Path) -> Result<PathBuf, String> {
                Ok(path.to_owned())
            }
        }
        assert!(server_address("http://h/mcp?q=1", None).is_err());
        let url = |c: Result<Connection, String>| c.map(|c| c.url);
        assert_eq!(
            url(client_connection(&Recorded, ConnectionOptions::NONE, true)),
            Ok("http://h/mcp?q=1".to_string())
        );
        assert_eq!(
            url(admin_connection(&Recorded, ConnectionOptions::NONE)),
            Ok("http://h/mcp?q=1".to_string())
        );
    }
}
