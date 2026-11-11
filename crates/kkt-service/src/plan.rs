//! What an installed service runs: the arguments it gives `kkt serve`, the
//! folders it needs, and the keys to make first. The plan is worked out
//! from plain inputs; the program reads the config file, finds its own
//! path, checks the settings and carries the plan out.
//!
//! The service runs `kkt serve` with absolute paths, since a service starts
//! in a working directory of the manager's choosing. The config file is
//! passed if there is one. The database and the access key file are passed
//! unless the config file names them, so the service never depends on a
//! default being worked out the same way later. A key is always in effect:
//! with none given, the default key file is used, and made if it is
//! missing.

use crate::Spec;
use kkt_admission::AccessKey;
use kkt_settings::InvalidSetting;
use kkt_settings::ServeOptions;
use std::path::{Path, PathBuf};

/// What the plan is worked out from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The config file the server reads, as an absolute path, if any.
    pub config_file: Option<PathBuf>,
    /// What the command line gives, with absolute paths.
    pub flags: ServeOptions,
    /// What the config file gives.
    pub from_file: ServeOptions,
    /// The per-user default database; `None` without a home directory.
    pub default_db: Option<PathBuf>,
    /// The per-user default access key file; `None` without a home
    /// directory.
    pub default_key: Option<PathBuf>,
}

/// Why no plan could be made.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// No database is given and there is no home directory for the
    /// default one.
    #[error("no home directory to keep the database in: set HOME or pass --db")]
    NoHomeForDb,
    /// No access key is given or turned off, and there is no home
    /// directory for the default one.
    #[error("no home directory to keep the access key in: set HOME or pass --access-key-file")]
    NoHomeForKey,
    /// A setting is bad. The caller says where it was set.
    #[error("invalid setting: {0}")]
    Invalid(InvalidSetting),
    /// A path the service is given is not valid UTF-8.
    #[error("{} is not valid UTF-8", .0.display())]
    NotText(PathBuf),
}

/// The plan for the service, before its arguments are written out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The access key file to make before installing, if any.
    pub new_key: Option<PathBuf>,
    /// The web key file to make before installing, if any.
    pub new_web_key: Option<PathBuf>,
    /// The settings to check now, so a bad one fails at install rather
    /// than in a loop of restarts. A key about to be made is good by
    /// construction, so it is left out.
    pub check: ServeOptions,
    /// The web key to check, in place of the one `check` resolves to.
    pub check_web_key: AccessKey,
    config_file: Option<PathBuf>,
    db: Option<PathBuf>,
    key: Option<PathBuf>,
    no_access_key: bool,
    dirs: Vec<PathBuf>,
}

/// Works out the plan. `exists` says whether a key file is already there.
pub fn plan(request: Request, exists: &dyn Fn(&Path) -> bool) -> Result<Plan, PlanError> {
    let Request {
        config_file,
        flags,
        from_file,
        default_db,
        default_key,
    } = request;
    let db = match (&flags.db, &from_file.db) {
        (Some(db), _) => Some(db.clone()),
        // Read from the config file each time the service starts.
        (None, Some(_)) => None,
        (None, None) => Some(default_db.clone().ok_or(PlanError::NoHomeForDb)?),
    };
    let key_off = flags.no_access_key.or(from_file.no_access_key) == Some(true);
    let (key, new_key) = match (&flags.access_key_file, &from_file.access_key_file) {
        (Some(key), _) => (Some(key.clone()), None),
        // Read from the config file each time the service starts.
        (None, Some(_)) => (None, None),
        // No key: nothing to pass or make. A key file given with it is
        // refused by the check.
        (None, None) if key_off => (None, None),
        (None, None) => {
            let key = default_key.clone().ok_or(PlanError::NoHomeForKey)?;
            let new = (!exists(&key)).then(|| key.clone());
            (Some(key), new)
        }
    };
    let mut check = flags.clone().over(from_file.clone());
    // The database is the one passed or the config file's, so it is always
    // known here: a missing home directory was refused above.
    let db_in_effect = match check.db.clone().or(default_db) {
        Some(db) => db,
        None => return Err(PlanError::NoHomeForDb),
    };
    let resolved = check
        .clone()
        .resolve(db_in_effect, default_key)
        .map_err(PlanError::Invalid)?;
    let new_web_key = match &resolved.web_key {
        AccessKey::Default(path) if !exists(path) => Some(path.clone()),
        _ => None,
    };
    match &new_key {
        Some(_) => {
            check.access_key_file = None;
            check.no_access_key = Some(true);
        }
        None => check.access_key_file = key.clone().or(check.access_key_file),
    }
    let check_web_key = if new_web_key.is_some() {
        AccessKey::None
    } else {
        resolved.web_key
    };
    // serve makes the database's folder only for the default database, so
    // make it here for whichever one the service will use.
    let dirs = resolved
        .db
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    Ok(Plan {
        new_key,
        new_web_key,
        check,
        check_web_key,
        config_file,
        db,
        key,
        // From the config file, serve reads it again each time it starts.
        no_access_key: flags.no_access_key == Some(true),
        dirs,
    })
}

impl Plan {
    /// The service that runs `program` as planned.
    pub fn spec(&self, program: PathBuf) -> Result<Spec, PlanError> {
        let text = |path: &Path| {
            path.to_str()
                .map(str::to_string)
                .ok_or_else(|| PlanError::NotText(path.to_path_buf()))
        };
        // The service is the server `kkt setup` meant, so it never stops
        // itself.
        let mut args = vec!["serve".to_string(), "--ignore-setup".to_string()];
        if let Some(config) = &self.config_file {
            args.extend(["--config".to_string(), text(config)?]);
        }
        if let Some(db) = &self.db {
            args.extend(["--db".to_string(), text(db)?]);
        }
        if let Some(key) = &self.key {
            args.extend(["--access-key-file".to_string(), text(key)?]);
        }
        if self.no_access_key {
            args.push("--no-access-key".to_string());
        }
        Ok(Spec {
            program,
            args,
            dirs: self.dirs.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = "/bin/kkt";
    const DEFAULT_DB: &str = "/home/u/data/kkt/kkt.db";
    const DEFAULT_KEY: &str = "/home/u/config/kkt/access-key";
    const DEFAULT_WEB_KEY: &str = "/home/u/config/kkt/web-key";

    fn path(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    /// Nothing given, with a home directory.
    fn request() -> Request {
        Request {
            config_file: None,
            flags: ServeOptions::NONE,
            from_file: ServeOptions::NONE,
            default_db: Some(path(DEFAULT_DB)),
            default_key: Some(path(DEFAULT_KEY)),
        }
    }

    fn none_exist(_: &Path) -> bool {
        false
    }

    fn all_exist(_: &Path) -> bool {
        true
    }

    fn args(plan: &Plan) -> Vec<String> {
        plan.spec(path(PROGRAM)).expect("text").args
    }

    #[test]
    fn with_nothing_given_the_defaults_are_passed_and_the_keys_made() {
        let plan = plan(request(), &none_exist).expect("plan");
        assert_eq!(
            plan.spec(path(PROGRAM)),
            Ok(Spec {
                program: path(PROGRAM),
                args: vec![
                    "serve".into(),
                    "--ignore-setup".into(),
                    "--db".into(),
                    DEFAULT_DB.into(),
                    "--access-key-file".into(),
                    DEFAULT_KEY.into(),
                ],
                dirs: vec![path("/home/u/data/kkt")],
            })
        );
        assert_eq!(plan.new_key, Some(path(DEFAULT_KEY)));
        assert_eq!(plan.new_web_key, Some(path(DEFAULT_WEB_KEY)));
        // The keys about to be made are left out of the check.
        assert_eq!(plan.check.access_key_file, None);
        assert_eq!(plan.check.no_access_key, Some(true));
        assert_eq!(plan.check_web_key, AccessKey::None);
    }

    #[test]
    fn existing_default_keys_are_passed_and_checked_not_made() {
        let plan = plan(request(), &all_exist).expect("plan");
        assert_eq!(
            args(&plan),
            [
                "serve",
                "--ignore-setup",
                "--db",
                DEFAULT_DB,
                "--access-key-file",
                DEFAULT_KEY
            ]
        );
        assert_eq!((plan.new_key, plan.new_web_key), (None, None));
        assert_eq!(plan.check.access_key_file, Some(path(DEFAULT_KEY)));
        assert_eq!(plan.check.no_access_key, None);
        assert_eq!(
            plan.check_web_key,
            AccessKey::Default(path(DEFAULT_WEB_KEY))
        );
    }

    #[test]
    fn a_missing_web_key_is_made_beside_an_existing_access_key() {
        let plan = plan(request(), &|p: &Path| p == Path::new(DEFAULT_KEY)).expect("plan");
        assert_eq!(plan.new_key, None);
        assert_eq!(plan.new_web_key, Some(path(DEFAULT_WEB_KEY)));
        assert_eq!(plan.check.access_key_file, Some(path(DEFAULT_KEY)));
        assert_eq!(plan.check_web_key, AccessKey::None);
    }

    #[test]
    fn a_missing_access_key_is_made_and_an_existing_web_key_still_checked() {
        let plan = plan(request(), &|p: &Path| p == Path::new(DEFAULT_WEB_KEY)).expect("plan");
        assert_eq!(plan.new_key, Some(path(DEFAULT_KEY)));
        assert_eq!(plan.new_web_key, None);
        assert_eq!(plan.check.no_access_key, Some(true));
        assert_eq!(
            plan.check_web_key,
            AccessKey::Default(path(DEFAULT_WEB_KEY))
        );
    }

    #[test]
    fn a_given_database_and_key_are_passed_and_the_key_never_made() {
        let request = Request {
            flags: ServeOptions {
                db: Some(path("/srv/kkt/my.db")),
                access_key_file: Some(path("/srv/keys/key")),
                ..ServeOptions::NONE
            },
            ..request()
        };
        let plan = plan(request, &none_exist).expect("plan");
        assert_eq!(
            args(&plan),
            [
                "serve",
                "--ignore-setup",
                "--db",
                "/srv/kkt/my.db",
                "--access-key-file",
                "/srv/keys/key"
            ]
        );
        assert_eq!(
            plan.spec(path(PROGRAM)).expect("text").dirs,
            [path("/srv/kkt")]
        );
        assert_eq!(plan.new_key, None);
        assert_eq!(plan.new_web_key, Some(path("/srv/keys/web-key")));
        assert_eq!(plan.check.access_key_file, Some(path("/srv/keys/key")));
    }

    #[test]
    fn what_the_config_file_gives_is_not_passed() {
        let request = Request {
            config_file: Some(path("/home/u/config/kkt/config.toml")),
            from_file: ServeOptions {
                db: Some(path("/data/file.db")),
                access_key_file: Some(path("/keys/key")),
                ..ServeOptions::NONE
            },
            ..request()
        };
        let plan = plan(request, &all_exist).expect("plan");
        assert_eq!(
            args(&plan),
            [
                "serve",
                "--ignore-setup",
                "--config",
                "/home/u/config/kkt/config.toml"
            ]
        );
        assert_eq!(
            plan.spec(path(PROGRAM)).expect("text").dirs,
            [path("/data")]
        );
        assert_eq!(plan.new_key, None);
        assert_eq!(plan.check.access_key_file, Some(path("/keys/key")));
        assert_eq!(
            plan.check_web_key,
            AccessKey::Default(path("/keys/web-key"))
        );
    }

    #[test]
    fn no_access_key_from_the_flags_is_passed_and_from_the_file_is_not() {
        let flags = Request {
            flags: ServeOptions {
                no_access_key: Some(true),
                ..ServeOptions::NONE
            },
            ..request()
        };
        let plan_flags = plan(flags, &none_exist).expect("plan");
        assert_eq!(
            args(&plan_flags),
            [
                "serve",
                "--ignore-setup",
                "--db",
                DEFAULT_DB,
                "--no-access-key"
            ]
        );
        assert_eq!((plan_flags.new_key, plan_flags.new_web_key), (None, None));
        assert_eq!(plan_flags.check_web_key, AccessKey::None);

        let file = Request {
            config_file: Some(path("/c.toml")),
            from_file: ServeOptions {
                no_access_key: Some(true),
                ..ServeOptions::NONE
            },
            ..request()
        };
        let plan_file = plan(file, &none_exist).expect("plan");
        assert_eq!(
            args(&plan_file),
            [
                "serve",
                "--ignore-setup",
                "--config",
                "/c.toml",
                "--db",
                DEFAULT_DB
            ]
        );
        assert_eq!((plan_file.new_key, plan_file.new_web_key), (None, None));
    }

    #[test]
    fn without_a_home_the_database_must_be_given_first() {
        let request = Request {
            default_db: None,
            default_key: None,
            ..request()
        };
        assert_eq!(plan(request, &none_exist), Err(PlanError::NoHomeForDb));
        assert_eq!(
            PlanError::NoHomeForDb.to_string(),
            "no home directory to keep the database in: set HOME or pass --db"
        );
    }

    #[test]
    fn without_a_home_the_key_must_be_given_or_turned_off() {
        let given_db = ServeOptions {
            db: Some(path("/srv/kkt.db")),
            ..ServeOptions::NONE
        };
        let request = Request {
            flags: given_db.clone(),
            default_db: None,
            default_key: None,
            ..request()
        };
        assert_eq!(
            plan(request.clone(), &none_exist),
            Err(PlanError::NoHomeForKey)
        );
        assert_eq!(
            PlanError::NoHomeForKey.to_string(),
            "no home directory to keep the access key in: set HOME or pass --access-key-file"
        );
        let off = Request {
            flags: ServeOptions {
                no_access_key: Some(true),
                ..given_db
            },
            ..request
        };
        let plan = plan(off, &none_exist).expect("plan");
        assert_eq!(
            args(&plan),
            [
                "serve",
                "--ignore-setup",
                "--db",
                "/srv/kkt.db",
                "--no-access-key"
            ]
        );
    }

    #[test]
    fn a_bad_setting_is_refused_with_its_name() {
        let bad_pack = Request {
            from_file: ServeOptions {
                name_pack: Some("missing".into()),
                ..ServeOptions::NONE
            },
            ..request()
        };
        match plan(bad_pack, &none_exist) {
            Err(PlanError::Invalid(e)) => assert_eq!(e.name, "name_pack"),
            other => panic!("{other:?}"),
        }
        // A key file given with no key is refused by the check, not here.
        let both = Request {
            flags: ServeOptions {
                no_access_key: Some(true),
                ..ServeOptions::NONE
            },
            from_file: ServeOptions {
                access_key_file: Some(path("/keys/key")),
                ..ServeOptions::NONE
            },
            ..request()
        };
        match plan(both, &none_exist) {
            Err(PlanError::Invalid(e)) => assert_eq!(e.name, "no_access_key"),
            other => panic!("{other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_text_is_refused_when_written_out() {
        use std::os::unix::ffi::OsStrExt;
        let db = PathBuf::from(std::ffi::OsStr::from_bytes(b"/srv/\xff.db"));
        let request = Request {
            flags: ServeOptions {
                db: Some(db.clone()),
                no_access_key: Some(true),
                ..ServeOptions::NONE
            },
            ..request()
        };
        let plan = plan(request, &none_exist).expect("plan");
        assert_eq!(
            plan.spec(path(PROGRAM)),
            Err(PlanError::NotText(db.clone()))
        );
        assert_eq!(
            PlanError::NotText(db).to_string(),
            "/srv/\u{fffd}.db is not valid UTF-8"
        );
    }
}
