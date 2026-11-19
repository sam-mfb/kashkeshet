//! `kkt key`: `create` makes a new access, web or admin key, `header`
//! prints the header clients send with the access key, and `show` prints a
//! key.

use crate::OperatorParts;
use crate::keys::{
    KeyKind, SelectedKeyError, admin_key_collision, local_operator_key, optional_key_path,
    runs_in_docker, selected_key_path,
};
use kkt_deployment::new_key_file;
use kkt_deployment::{
    AdminKeyCollision, AdminNameLineError, CreateRefusal, FindKeyError, NameNotWritten,
};
use kkt_deployment::{admin_name_line, admin_name_written};
use kkt_llm_agent_config as clients;
use kkt_names::{AdminName, MAX_ADMIN_NAME_CHARS, NameError};
use std::io;
use std::path::{Path, PathBuf};

/// Why `kkt key header`, `show` or `create` failed: a source's own error,
/// or the step here that failed, with the key file it was on.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error(transparent)]
    Find(#[from] FindKeyError),
    #[error(transparent)]
    Selected(#[from] SelectedKeyError),
    #[error(transparent)]
    Header(#[from] clients::ClientConfigError),
    #[error(transparent)]
    AdminNameLine(#[from] AdminNameLineError),
    #[error(transparent)]
    Refused(#[from] CreateRefusal),
    /// The admin name given is not a valid one.
    #[error(transparent)]
    Name(#[from] NameError),
    #[error(transparent)]
    Collision(#[from] AdminKeyCollision),
    #[error(transparent)]
    Write(#[from] kkt_keys::KeyFileError),
    #[error(transparent)]
    NameNotWritten(#[from] NameNotWritten),
    #[error("could not read key file {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not tell whether the key file {} is there: {source}", path.display())]
    Exists {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not resolve key file {}: {source}", path.display())]
    Resolve {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("set HOME or pass --path")]
    NoHome,
}

pub fn header(
    parts: &OperatorParts<'_>,
    path: Option<PathBuf>,
    json: bool,
) -> Result<(), KeyError> {
    let path = selected_key_path(parts, path, KeyKind::Access)?;
    let text = parts
        .key_files
        .read(&path)
        .map_err(|source| KeyError::Read {
            path: path.clone(),
            source,
        })?;
    parts
        .output
        .output(format_args!("{}\n", clients::header(text.trim(), json)?));
    Ok(())
}

/// Prints the selected key. A missing admin key is not an error but has its
/// own exit code, [`kkt_keys::NO_ADMIN_KEY_EXIT`], so `kkt setup` and
/// `kkt upgrade` can tell it from a key they failed to read.
pub fn show(
    parts: &OperatorParts<'_>,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<std::process::ExitCode, KeyError> {
    let Some(path) = optional_key_path(parts, path, kind)? else {
        parts.output.errors(format_args!(
            "there is no admin key; `kkt key create --admin` makes one\n"
        ));
        return Ok(std::process::ExitCode::from(kkt_keys::NO_ADMIN_KEY_EXIT));
    };
    let text = match parts.key_files.read(&path) {
        Ok(text) => text,
        Err(e) if kind == KeyKind::Admin && e.kind() == std::io::ErrorKind::NotFound => {
            parts.output.errors(format_args!(
                "there is no admin key at {}; `kkt key create --admin` makes one\n",
                path.display()
            ));
            return Ok(std::process::ExitCode::from(kkt_keys::NO_ADMIN_KEY_EXIT));
        }
        Err(source) => return Err(KeyError::Read { path, source }),
    };
    parts.output.output(format_args!("{}\n", text.trim()));
    if kind == KeyKind::Admin {
        // On standard error, so standard output stays the key alone.
        if let Some(line) = admin_name_line(parts.key_files, &path, &text, "admin name")? {
            parts.output.errors(format_args!("{line}\n"));
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

/// The key `kkt key create` makes: its kind, the file to write it to if
/// given, and the admin agent's name if given.
pub struct NewKey<'a> {
    pub kind: KeyKind,
    pub path: Option<PathBuf>,
    pub name: Option<&'a str>,
}

pub fn create(
    parts: &OperatorParts<'_>,
    new: NewKey<'_>,
    force: bool,
    show: bool,
) -> Result<(), KeyError> {
    let NewKey { kind, path, name } = new;
    // Checked before anything is written, so a refused key or a bad name
    // leaves no key.
    let in_docker =
        kkt_deployment::create_asks_for_docker(kind, path.is_some()) && runs_in_docker(parts)?;
    let name = checked_name(kind, in_docker, name)?;
    if kind == KeyKind::Admin
        && let Some(destination) = path.as_ref()
        && let Some(access) = local_operator_key(parts, None, None, KeyKind::Access)?
    {
        // Canonicalize existing files to also catch relative paths and
        // symlinks; a missing destination cannot overwrite an existing key.
        let there = |path: &Path| {
            parts
                .key_files
                .try_exists(path)
                .map_err(|source| KeyError::Exists {
                    path: path.to_path_buf(),
                    source,
                })
        };
        let (destination, access) = if there(destination)? && there(&access)? {
            let resolve = |path: &Path| {
                parts
                    .paths
                    .canonicalize(path)
                    .map_err(|source| KeyError::Resolve {
                        path: path.to_path_buf(),
                        source,
                    })
            };
            (resolve(destination)?, resolve(&access)?)
        } else {
            let resolve = |path: &Path| {
                parts
                    .paths
                    .absolute(path)
                    .map_err(|source| KeyError::Resolve {
                        path: path.to_path_buf(),
                        source,
                    })
            };
            (resolve(destination)?, resolve(&access)?)
        };
        kkt_deployment::create_destination_check(&destination, &access)?;
    }
    let path = match kind {
        KeyKind::Access => path
            .or_else(|| parts.serve.default_access_key())
            .ok_or(KeyError::NoHome)?,
        _ => optional_key_path(parts, path, kind)?.ok_or_else(admin_key_collision)?,
    };
    let key = new_key_file(parts.key_files, parts.entropy, &path, force)?;
    match kind {
        KeyKind::Web => parts.output.output(format_args!(
            "Wrote a new web key to {}. Restart the server to use it and end existing web sessions.\n",
            path.display()
        )),
        KeyKind::Admin => {
            parts.output.output(format_args!(
                "Wrote a new admin key to {}. Restart the server to use it: /mcp-admin then takes only this key. Read it with `kkt key show --admin`.\n",
                path.display()
            ));
            parts.output.output(format_args!("{}\n", admin_name_written(
                    &path,
                    name.as_ref().map(ToString::to_string).as_deref(),
                    write_admin_name(parts.key_files, &path, &key, name.as_ref()),
                )?));
        }
        KeyKind::Access => {}
    }
    if kind != KeyKind::Access {
        if show {
            parts.output.output(format_args!("{key}\n"));
        }
        return Ok(());
    }
    let shown = path.display();
    parts
        .output
        .output(format_args!("Wrote a new access key to {shown}\n"));
    parts.output.output(format_args!("\n"));
    parts
        .output
        .output(format_args!("Start the server with it:\n"));
    parts
        .output
        .output(format_args!("  kkt serve --access-key-file {shown}\n"));
    parts
        .output
        .output(format_args!("or put this line in the config file:\n"));
    parts
        .output
        .output(format_args!("  access_key_file = \"{shown}\"\n"));
    parts.output.output(format_args!("\n"));
    parts.output.output(format_args!(
        "Clients send it as the header `Authorization: Bearer <key>`. For example:\n"
    ));
    parts
        .output
        .output(format_args!("  export KKT_ACCESS_KEY=\"$(cat {shown})\"\n"));
    if show {
        parts.output.output(format_args!("\n"));
        parts.output.output(format_args!("The key:\n"));
        parts.output.output(format_args!("{key}\n"));
    }
    Ok(())
}

/// What `kkt key create` checks of its key and name before it writes
/// anything, in order: kkt-deployment's `create_checks`, then whether `name`
/// is a valid admin name. Returns the admin name to tie to the new key.
fn checked_name(
    kind: KeyKind,
    in_docker: bool,
    name: Option<&str>,
) -> Result<Option<AdminName>, KeyError> {
    kkt_deployment::create_checks(kind, in_docker, name)?;
    Ok(name
        .map(|name| AdminName::new(name, MAX_ADMIN_NAME_CHARS))
        .transpose()?)
}

/// Ties `name` to the new admin key `key`, just written to the file at
/// `path`, in the name file beside it. The name file holds a hash of the
/// key, so a name never passes to another key: one left from an earlier key
/// names nothing. With no name, such a file is removed. Returns how the
/// write went; the caller words it.
fn write_admin_name(
    files: &dyn kkt_keys::KeyFiles,
    path: &Path,
    key: &str,
    name: Option<&AdminName>,
) -> std::io::Result<()> {
    let bound = name.map(|name| kkt_keys::bound_to(name, key));
    files.write_admin_name(path, bound.as_deref())
}

#[cfg(test)]
mod tests {
    //! `kkt key create`'s own checks of the name and its name file, pinned:
    //! the order and texts of the refusals, and what the name file holds.

    use super::*;
    use kkt_keys::testing::Memory;

    #[test]
    fn key_create_checks_the_name_after_the_key() {
        let checked =
            |kind, in_docker, name| checked_name(kind, in_docker, name).map_err(|e| e.to_string());
        assert_eq!(
            checked(KeyKind::Admin, true, Some("Bad Name")),
            Err(
                "make or rotate the admin key inside the container: `docker compose exec kkt \
                 kkt key create --admin --force --name <name>`, then restart kkt and rerun \
                 setup with --admin-key to save the host copy"
                    .to_string()
            )
        );
        for kind in [KeyKind::Access, KeyKind::Web] {
            assert_eq!(
                checked(kind, false, Some("sentinel")),
                Err("--name names the admin agent, so it goes with --admin".to_string())
            );
            assert_eq!(checked(kind, false, None), Ok(None));
        }
        assert_eq!(
            checked(KeyKind::Admin, false, Some("sentinel")),
            Ok(Some(
                AdminName::new("admin-sentinel", MAX_ADMIN_NAME_CHARS).unwrap()
            ))
        );
        assert_eq!(checked(KeyKind::Admin, false, None), Ok(None));
        assert_eq!(
            checked(KeyKind::Admin, false, Some("Bad Name")),
            Err(
                "the admin name must be lowercase letters and digits, in words joined by single \
                 hyphens, such as `sentinel` (which becomes `admin-sentinel`); got \"Bad Name\""
                    .to_string()
            )
        );
    }

    #[test]
    fn an_admin_name_is_bound_to_its_key_and_removed_with_none() {
        let files = Memory::new().with_dir("/k");
        let path = Path::new("/k/admin-key");
        let name = AdminName::new("sentinel", MAX_ADMIN_NAME_CHARS).expect("a name");
        assert_eq!(
            write_admin_name(&files, path, "the-key", Some(&name)).map_err(|e| e.to_string()),
            Ok(())
        );
        let names = kkt_keys::admin_name_path(path);
        assert_eq!(
            files.file(&names).expect("a name file").text,
            kkt_keys::bound_to(&name, "the-key")
        );
        assert_eq!(
            write_admin_name(&files, path, "the-key", None).map_err(|e| e.to_string()),
            Ok(())
        );
        assert_eq!(files.file(&names), None);
    }

    /// A machine for the refusals' pin: nothing installed, the serve
    /// config's access key file and default key as given, key files and
    /// paths that refuse as given, and no database.
    struct Machine {
        configured: Option<PathBuf>,
        default_key: Option<PathBuf>,
        paths_refuse: bool,
        entropy: kkt_entropy::testing::SeqEntropy,
    }

    impl crate::ServeConfig for Machine {
        fn config(&self, _: Option<PathBuf>) -> Result<crate::Config, crate::PartError> {
            Ok(crate::Config {
                db: None,
                max_lane_chars: 100,
            })
        }
        fn configured_access_key(&self) -> Result<Option<PathBuf>, crate::PartError> {
            Ok(self.configured.clone())
        }
        fn database(
            &self,
            given: Option<PathBuf>,
            _: Option<PathBuf>,
        ) -> Result<PathBuf, crate::PartError> {
            Ok(given.unwrap_or_else(|| PathBuf::from("/db/kkt.db")))
        }
        fn default_access_key(&self) -> Option<PathBuf> {
            self.default_key.clone()
        }
        fn config_dir(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/k"))
        }
    }

    impl crate::InstalledHere for Machine {
        fn recorded(&self) -> Result<Option<kkt_deployment::ServerKeys>, crate::PartError> {
            Ok(None)
        }
        fn unfinished(&self) -> Result<Option<kkt_deployment::UnfinishedSetup>, crate::PartError> {
            Ok(None)
        }
        fn unrecorded(&self) -> Result<Option<kkt_deployment::ServerKeys>, crate::PartError> {
            Ok(None)
        }
    }

    impl crate::Paths for Machine {
        fn absolute(&self, path: &Path) -> std::io::Result<PathBuf> {
            if self.paths_refuse {
                return Err(std::io::Error::other("refused"));
            }
            Ok(path.to_path_buf())
        }
        fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
            self.absolute(path)
        }
    }

    impl crate::Databases for Machine {
        fn exists(&self, _: &Path) -> bool {
            false
        }
        fn open(&self, _: &Path) -> Result<kkt_rules::Db, crate::PartError> {
            unreachable!("there is no database")
        }
    }

    impl crate::lanes::LaneServers for Machine {
        fn here(
            &self,
            _: Option<PathBuf>,
            _: Option<PathBuf>,
        ) -> Result<Box<dyn crate::lanes::LaneServer + '_>, crate::PartError> {
            unreachable!("no lanes here")
        }
    }

    impl kkt_lane_files::Disk for Machine {
        fn file_names(&self, _: &Path) -> std::io::Result<Vec<String>> {
            unreachable!("no lanes here")
        }
        fn read(&self, _: &Path) -> std::io::Result<String> {
            unreachable!("no lanes here")
        }
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            unreachable!("no lanes here")
        }
        fn write(&self, _: &Path, _: &str) -> std::io::Result<()> {
            unreachable!("no lanes here")
        }
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            unreachable!("no lanes here")
        }
    }

    impl kkt_lane_files::LaneCommit for Machine {
        fn commit(&self, _: &Path) -> std::io::Result<Option<String>> {
            unreachable!("no lanes here")
        }
    }

    impl crate::Output for Machine {
        fn output(&self, _: std::fmt::Arguments<'_>) {}
        fn errors(&self, _: std::fmt::Arguments<'_>) {}
    }

    fn parts<'a>(machine: &'a Machine, key_files: &'a Memory) -> OperatorParts<'a> {
        OperatorParts {
            installed: machine,
            serve: machine,
            paths: machine,
            key_files,
            entropy: &machine.entropy,
            lane_servers: machine,
            lanes: machine,
            lane_commit: machine,
            database: machine,
            clock: std::sync::Arc::new(kkt_clock::testing::FixedClock::new(
                kkt_clock::Time::from_parts(2026, 11, 18, 12, 0, 0).expect("a time"),
            )),
            output: machine,
        }
    }

    /// The refusals `kkt key` and `kkt agent` word themselves, word for
    /// word.
    #[test]
    fn the_key_and_agent_refusals_are_pinned() {
        let machine = |configured: Option<&str>, default_key: Option<&str>, paths_refuse| Machine {
            configured: configured.map(PathBuf::from),
            default_key: default_key.map(PathBuf::from),
            paths_refuse,
            entropy: kkt_entropy::testing::SeqEntropy::new(0),
        };
        let access = Some("/k/access-key");
        fn said(result: Result<(), KeyError>) -> Result<(), String> {
            result.map_err(|e| e.to_string())
        }

        let here = machine(access, None, false);
        let files = Memory::new().failing("/k/refused");
        assert_eq!(
            said(header(
                &parts(&here, &files),
                Some("/k/missing".into()),
                false
            )),
            Err("could not read key file /k/missing: no such file".to_string())
        );
        assert_eq!(
            show(
                &parts(&here, &files),
                Some("/k/refused".into()),
                KeyKind::Access
            )
            .map(|_| ())
            .map_err(|e| e.to_string()),
            Err("could not read key file /k/refused: refused".to_string())
        );

        let keys_off = machine(None, None, false);
        assert_eq!(
            said(header(&parts(&keys_off, &Memory::new()), None, false)),
            Err("keys are disabled in the serve config".to_string())
        );
        // An access key in the admin key's place leaves no admin key.
        let collides = machine(Some("/k/admin-key"), None, false);
        assert_eq!(
            crate::keys::selected_key_path(&parts(&collides, &Memory::new()), None, KeyKind::Admin)
                .map_err(|e| e.to_string()),
            Err("there is no admin key".to_string())
        );

        let new_admin = |at: &str| NewKey {
            kind: KeyKind::Admin,
            path: Some(at.into()),
            name: None,
        };
        let refusing = Memory::new()
            .with_dir("/k")
            .with_file("/k/access-key", "k\n")
            .failing("/k/refused");
        assert_eq!(
            said(create(
                &parts(&here, &refusing),
                new_admin("/k/refused"),
                false,
                false
            )),
            Err("could not tell whether the key file /k/refused is there: refused".to_string())
        );
        let both_there = Memory::new()
            .with_dir("/k")
            .with_file("/k/access-key", "k\n")
            .with_file("/k/admin", "old\n");
        let unresolved = machine(access, None, true);
        assert_eq!(
            said(create(
                &parts(&unresolved, &both_there),
                new_admin("/k/admin"),
                true,
                false
            )),
            Err("could not resolve key file /k/admin: refused".to_string())
        );
        let one_there = Memory::new()
            .with_dir("/k")
            .with_file("/k/access-key", "k\n");
        assert_eq!(
            said(create(
                &parts(&unresolved, &one_there),
                new_admin("/k/admin"),
                false,
                false
            )),
            Err("could not resolve key file /k/admin: refused".to_string())
        );

        let homeless = machine(access, None, false);
        let new_access = NewKey {
            kind: KeyKind::Access,
            path: None,
            name: None,
        };
        assert_eq!(
            said(create(
                &parts(&homeless, &Memory::new()),
                new_access,
                false,
                false
            )),
            Err("set HOME or pass --path".to_string())
        );

        assert_eq!(
            crate::lanes::existing_db(&parts(&here, &Memory::new()).lane_parts(), None, None)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err("no database at /db/kkt.db; pass --db with the server's database file".to_string())
        );
    }
}
