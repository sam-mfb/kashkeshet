//! Which key files a local `kkt backup` holds beside the database and the
//! config file: the access key, the web key, the admin key and the admin
//! name. These are backup's own rules, not serve's: a backup copies what is
//! there and makes nothing. The program says whether each file exists, and
//! copies the files decided here.

use kkt_admission::{admin_key_path, admin_name_path, web_key_path};
use std::path::{Path, PathBuf};

/// What the program found when it looked for a file: whether it exists, or
/// the error inspecting it.
pub type Found = Result<bool, String>;

/// Where the access key may be, from the serve config and the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessKeyFrom<'a> {
    /// The config file's `access_key_file`.
    pub configured: Option<&'a Path>,
    /// The config file's `no_access_key`.
    pub no_access_key: Option<bool>,
    /// The default access key file, `None` without a home.
    pub default: Option<&'a Path>,
}

/// The key files a backup holds. Each is `None` when the backup leaves it
/// out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupKeys {
    pub access_key: Option<PathBuf>,
    pub web_key: Option<PathBuf>,
    pub admin_key: Option<PathBuf>,
    pub admin_name: Option<PathBuf>,
}

/// The key files a backup holds, given where the access key may be
/// (`from`). `found` answers whether a file exists; it is asked only about
/// the files these rules need, in this order:
///
/// - The access key is the configured one, even if it is missing; none if
///   `no_access_key` is set; else the default one, only if it exists. An
///   error inspecting the default counts as missing.
/// - The web key beside the access key, only if it exists: installations
///   from before separate web keys have none until their first start or
///   service upgrade.
/// - The admin key beside the access key, only if it exists: only servers
///   whose operator asked for one have it.
/// - The admin key's name file, only if there is an admin key and the file
///   exists: only an admin key made with a name has one.
///
/// An error inspecting the web key, the admin key or the name file stops
/// the backup.
pub fn backup_keys(
    from: AccessKeyFrom<'_>,
    found: &mut dyn FnMut(&Path) -> Found,
) -> Result<BackupKeys, String> {
    let access_key = match (from.configured, from.no_access_key) {
        (Some(path), _) => Some(path.to_path_buf()),
        (None, Some(true)) => None,
        (None, _) => from
            .default
            .filter(|path| matches!(found(path), Ok(true)))
            .map(Path::to_path_buf),
    };
    let web_key = present(access_key.as_deref().map(web_key_path), found)?;
    let admin_key = present(access_key.as_deref().and_then(admin_key_path), found)?;
    let admin_name = present(admin_key.as_deref().map(admin_name_path), found)?;
    Ok(BackupKeys {
        access_key,
        web_key,
        admin_key,
        admin_name,
    })
}

/// `candidate`, if it exists.
fn present(
    candidate: Option<PathBuf>,
    found: &mut dyn FnMut(&Path) -> Found,
) -> Result<Option<PathBuf>, String> {
    let Some(path) = candidate else {
        return Ok(None);
    };
    match found(&path) {
        Ok(true) => Ok(Some(path)),
        Ok(false) => Ok(None),
        Err(e) => Err(format!("could not inspect {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// The files a test machine has, and the paths `found` was asked about.
    struct Machine {
        files: BTreeMap<PathBuf, Found>,
        asked: Vec<PathBuf>,
    }

    impl Machine {
        fn with(files: &[(&str, Found)]) -> Machine {
            Machine {
                files: files
                    .iter()
                    .map(|(path, found)| (PathBuf::from(path), found.clone()))
                    .collect(),
                asked: Vec::new(),
            }
        }

        fn keys(&mut self, from: AccessKeyFrom<'_>) -> Result<BackupKeys, String> {
            backup_keys(from, &mut |path| {
                self.asked.push(path.to_path_buf());
                self.files.get(path).cloned().unwrap_or(Ok(false))
            })
        }

        fn asked(&self) -> Vec<&str> {
            self.asked.iter().map(|p| p.to_str().unwrap()).collect()
        }
    }

    const DEFAULT: &str = "/home/k/.config/kkt/access-key";

    fn from_default() -> AccessKeyFrom<'static> {
        AccessKeyFrom {
            configured: None,
            no_access_key: None,
            default: Some(Path::new(DEFAULT)),
        }
    }

    fn configured(path: &'static str) -> AccessKeyFrom<'static> {
        AccessKeyFrom {
            configured: Some(Path::new(path)),
            ..from_default()
        }
    }

    fn path(path: &str) -> Option<PathBuf> {
        Some(PathBuf::from(path))
    }

    fn only_access(access: Option<PathBuf>) -> BackupKeys {
        BackupKeys {
            access_key: access,
            web_key: None,
            admin_key: None,
            admin_name: None,
        }
    }

    #[test]
    fn a_configured_access_key_is_held_even_if_missing() {
        let mut machine = Machine::with(&[]);
        assert_eq!(
            machine.keys(configured("/keys/access-key")),
            Ok(only_access(path("/keys/access-key")))
        );
        assert_eq!(machine.asked(), ["/keys/web-key", "/keys/admin-key"]);
    }

    #[test]
    fn a_configured_access_key_wins_over_no_access_key() {
        let mut machine = Machine::with(&[]);
        let from = AccessKeyFrom {
            no_access_key: Some(true),
            ..configured("/keys/access-key")
        };
        assert_eq!(
            machine.keys(from),
            Ok(only_access(path("/keys/access-key")))
        );
    }

    #[test]
    fn no_access_key_means_no_keys_and_nothing_inspected() {
        let mut machine = Machine::with(&[(DEFAULT, Ok(true))]);
        let from = AccessKeyFrom {
            no_access_key: Some(true),
            ..from_default()
        };
        assert_eq!(machine.keys(from), Ok(only_access(None)));
        assert!(machine.asked.is_empty());
    }

    #[test]
    fn the_default_access_key_is_held_only_if_it_exists() {
        let mut there = Machine::with(&[(DEFAULT, Ok(true))]);
        assert_eq!(there.keys(from_default()), Ok(only_access(path(DEFAULT))));
        let mut missing = Machine::with(&[]);
        assert_eq!(missing.keys(from_default()), Ok(only_access(None)));
        assert_eq!(missing.asked(), [DEFAULT]);
        let off = AccessKeyFrom {
            no_access_key: Some(false),
            ..from_default()
        };
        assert_eq!(there.keys(off), Ok(only_access(path(DEFAULT))));
    }

    #[test]
    fn an_error_inspecting_the_default_access_key_counts_as_missing() {
        let mut machine = Machine::with(&[(DEFAULT, Err("permission denied".into()))]);
        assert_eq!(machine.keys(from_default()), Ok(only_access(None)));
    }

    #[test]
    fn without_a_home_there_is_no_default_access_key() {
        let mut machine = Machine::with(&[]);
        let from = AccessKeyFrom {
            default: None,
            ..from_default()
        };
        assert_eq!(machine.keys(from), Ok(only_access(None)));
        assert!(machine.asked.is_empty());
    }

    #[test]
    fn every_key_that_exists_is_held() {
        let mut machine = Machine::with(&[
            ("/keys/web-key", Ok(true)),
            ("/keys/admin-key", Ok(true)),
            ("/keys/admin-key.name", Ok(true)),
        ]);
        assert_eq!(
            machine.keys(configured("/keys/access-key")),
            Ok(BackupKeys {
                access_key: path("/keys/access-key"),
                web_key: path("/keys/web-key"),
                admin_key: path("/keys/admin-key"),
                admin_name: path("/keys/admin-key.name"),
            })
        );
        assert_eq!(
            machine.asked(),
            ["/keys/web-key", "/keys/admin-key", "/keys/admin-key.name"]
        );
    }

    #[test]
    fn a_missing_web_key_is_left_out() {
        let mut machine = Machine::with(&[("/keys/admin-key", Ok(true))]);
        let keys = machine.keys(configured("/keys/access-key")).unwrap();
        assert_eq!(keys.web_key, None);
        assert_eq!(keys.admin_key, path("/keys/admin-key"));
    }

    #[test]
    fn the_admin_name_needs_the_admin_key() {
        let mut machine = Machine::with(&[("/keys/admin-key.name", Ok(true))]);
        let keys = machine.keys(configured("/keys/access-key")).unwrap();
        assert_eq!(keys.admin_key, None);
        assert_eq!(keys.admin_name, None);
        assert_eq!(machine.asked(), ["/keys/web-key", "/keys/admin-key"]);
    }

    #[test]
    fn an_admin_key_without_a_name_file_has_no_name() {
        let mut machine = Machine::with(&[("/keys/admin-key", Ok(true))]);
        let keys = machine.keys(configured("/keys/access-key")).unwrap();
        assert_eq!(keys.admin_key, path("/keys/admin-key"));
        assert_eq!(keys.admin_name, None);
    }

    #[test]
    fn an_access_key_in_the_admin_keys_place_has_no_admin_key() {
        let mut machine = Machine::with(&[("/keys/admin-key", Ok(true))]);
        assert_eq!(
            machine.keys(configured("/keys/admin-key")),
            Ok(only_access(path("/keys/admin-key")))
        );
        assert_eq!(machine.asked(), ["/keys/web-key"]);
    }

    #[test]
    fn an_error_inspecting_a_key_stops_the_backup() {
        for (file, earlier) in [
            ("/keys/web-key", vec![]),
            ("/keys/admin-key", vec![("/keys/web-key", Ok(true))]),
            ("/keys/admin-key.name", vec![("/keys/admin-key", Ok(true))]),
        ] {
            let mut files = earlier;
            files.push((file, Err("permission denied".into())));
            let mut machine = Machine::with(&files);
            assert_eq!(
                machine.keys(configured("/keys/access-key")),
                Err(format!("could not inspect {file}: permission denied"))
            );
        }
    }
}
