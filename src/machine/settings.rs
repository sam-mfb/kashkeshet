//! The settings `kkt serve` would run with: reading the config file, where
//! each setting came from, and the startup checks.

use crate::machine::keys::{AdminNameFile, read_admin_key, read_admin_name, read_key};
use kkt_admission::{Admission, SharedKey};
use kkt_config::{self as config, InvalidSetting, LogLevel};
use kkt_hosts::AllowedHosts;
use kkt_server::AppConfig;
use kkt_settings::{self as settings, AccessKey, Env, Platform, ServeOptions, ServeSettings};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Reads the arguments a service file gives `kkt` as a `kkt serve` command:
/// the config file it names and the settings it gives. The command line is
/// `cli`'s, so `cli` supplies it ([`cli::serve_args`]).
pub type ServeArgs = fn(&[String]) -> Result<(Option<PathBuf>, ServeOptions), String>;

/// The address `kkt serve` would listen on with the default config file.
pub fn serve_bind(env: &Env) -> Result<SocketAddr, String> {
    Ok(serve_settings(env, None, ServeOptions::NONE)?.bind)
}

/// The settings `kkt serve` would run with, given `config` and `flags`,
/// without making anything.
pub(crate) fn serve_settings(
    env: &Env,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<ServeSettings, String> {
    let loaded = load_serve_options(env, config, flags)?;
    let sources = loaded.sources();
    loaded
        .flags
        .over(loaded.from_file)
        .resolve(
            env.default_db(Platform::CURRENT).unwrap_or_default(),
            env.access_key_file(),
        )
        .map_err(|e| sources.invalid(e))
}

/// The settings `kkt serve` would run with, given `args` after `kkt`, as a
/// service file passes them.
pub(crate) fn serve_settings_from_args(
    env: &Env,
    serve_args: ServeArgs,
    args: &[String],
) -> Result<ServeSettings, String> {
    let (config, options) = serve_args(args)?;
    serve_settings(env, config, options)
}

/// `settings` as a running server sees them: a default access key file is
/// the same key as that file given by name.
pub(crate) fn as_running(settings: ServeSettings) -> ServeSettings {
    match settings.access_key {
        AccessKey::Default(path) => ServeSettings {
            access_key: AccessKey::File(path),
            ..settings
        },
        _ => settings,
    }
}

/// What `kkt serve` reads before applying defaults: the config file, if
/// any, what it gives, and what the flags give.
pub struct Loaded {
    pub config_file: Option<PathBuf>,
    pub from_file: ServeOptions,
    pub flags: ServeOptions,
}

impl Loaded {
    pub fn sources(&self) -> Sources {
        Sources {
            file: self
                .config_file
                .clone()
                .map(|path| (path, self.from_file.clone())),
            flags: self.flags.clone(),
        }
    }
}

/// Reads the config file `kkt serve` would read, and puts it with `flags`.
pub fn load_serve_options(
    env: &Env,
    config: Option<PathBuf>,
    flags: ServeOptions,
) -> Result<Loaded, String> {
    let (config_file, from_file) = read_config(env, config)?;
    Ok(Loaded {
        config_file,
        from_file,
        flags,
    })
}

/// The config file to use (`config`, else the default one if it exists),
/// and what it gives.
pub fn read_config(
    env: &Env,
    config: Option<PathBuf>,
) -> Result<(Option<PathBuf>, ServeOptions), String> {
    let config_file = match config {
        Some(path) => Some(path),
        None => env.config_file().filter(|path| path.exists()),
    };
    let from_file = match &config_file {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                format!(
                    "invalid setting: config could not be read from {}: {e}",
                    path.display()
                )
            })?;
            let dir = path.parent().unwrap_or(Path::new("."));
            settings::parse_file(&text, dir, env.home.as_deref())
                .map_err(|e| format!("invalid setting: {e} (in {})", path.display()))?
        }
        None => ServeOptions::NONE,
    };
    Ok((config_file, from_file))
}

/// Where a server's settings came from, so a bad one can be traced to the
/// config file when that is where it was set.
pub struct Sources {
    pub(crate) flags: ServeOptions,
    /// The config file read, and what it gave.
    pub(crate) file: Option<(PathBuf, ServeOptions)>,
}

impl Sources {
    /// The message for a bad setting, naming the config file if the setting
    /// came from there.
    pub fn invalid(&self, e: InvalidSetting) -> String {
        match &self.file {
            Some((path, file)) if file.gives(e.name) && !self.flags.gives(e.name) => {
                format!("invalid setting: {e} (set in {})", path.display())
            }
            _ => format!("invalid setting: {e}"),
        }
    }
}

/// Settings that passed every startup check, with the files they name read.
pub struct Checked {
    pub db_path: PathBuf,
    pub bind: SocketAddr,
    pub config: AppConfig,
    // The rest only the record of the server run needs.
    pub allow_any_bind: bool,
    /// The config file the settings were read from, if any.
    pub config_file: Option<String>,
    pub log_file: Option<String>,
    pub log_level: LogLevel,
    pub dependency_log_level: LogLevel,
}

/// Runs the startup checks on `settings` and reads the files they name.
/// The errors name the bad setting, and the config file if it set it.
pub fn check_settings(settings: ServeSettings, sources: &Sources) -> Result<Checked, String> {
    let ServeSettings {
        db: db_path,
        bind,
        allow_any_bind,
        limits,
        allow_origins,
        allow_hosts,
        instructions,
        access_key,
        web_key,
        admin_key,
        log_file,
        log_level,
        dependency_log_level,
    } = settings;
    config::check_bind(bind, allow_any_bind, access_key != AccessKey::None)
        .map_err(|e| sources.invalid(e))?;
    config::check_dependency_log_level(dependency_log_level).map_err(|e| sources.invalid(e))?;
    limits.check().map_err(|e| sources.invalid(e))?;
    let allowed_hosts = AllowedHosts::new(&allow_hosts).map_err(|e| sources.invalid(e))?;
    let instructions = match &instructions {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?,
        None => kkt_mcp::DEFAULT_INSTRUCTIONS.to_string(),
    };
    // Only reads the key: `serve` makes a missing default key first, and
    // `kkt service install` checks settings without making anything.
    let admission = match access_key {
        AccessKey::None => Admission::Open,
        AccessKey::File(path) | AccessKey::Default(path) => {
            Admission::SharedKey(read_key(&path, sources)?)
        }
    };
    let web_admission = match web_key {
        AccessKey::None => Admission::Open,
        AccessKey::File(path) | AccessKey::Default(path) => {
            let text = std::fs::read_to_string(&path).map_err(|e| {
                format!(
                    "invalid setting: web_key_file could not be read from {}: {e}",
                    path.display()
                )
            })?;
            if let Admission::SharedKey(access) = &admission
                && access.matches(text.trim().as_bytes())
            {
                return Err("invalid setting: web_key_file must contain a different key from access_key_file".into());
            }
            let key = SharedKey::new(&text, path.display().to_string())
                .map_err(|e| format!("invalid setting: web_key_file: {}", e.rule))?;
            Admission::SharedKey(key)
        }
    };
    let (admin_key, admin_name) = match admin_key {
        Some(path) => match read_admin_key(&path, &[&admission, &web_admission])? {
            Some((key, text)) => (
                Some(key),
                match read_admin_name(&path, &text)? {
                    AdminNameFile::Named(name) => Some(name),
                    // A name file for a different key names nothing.
                    AdminNameFile::None | AdminNameFile::OtherKey(_) => None,
                },
            ),
            None => (None, None),
        },
        None => (None, None),
    };
    let config = AppConfig {
        admission,
        web_admission,
        admin_key,
        admin_name,
        limits,
        allowed_origins: allow_origins,
        allowed_hosts,
        instructions,
    };
    Ok(Checked {
        db_path,
        bind,
        config,
        allow_any_bind,
        config_file: sources.file.as_ref().map(|(p, _)| p.display().to_string()),
        log_file: log_file.map(|p| p.display().to_string()),
        log_level,
        dependency_log_level,
    })
}

/// An environment with only `home` set, for tests.
#[cfg(test)]
pub(crate) fn test_env(home: Option<&Path>) -> Env {
    Env {
        home: home.map(Path::to_path_buf),
        claude_config_dir: None,
        codex_home: None,
        xdg_config_home: None,
        xdg_data_home: None,
    }
}

/// Writes `text` as the default config file under `home`, and returns its
/// path, for tests.
#[cfg(test)]
pub(crate) fn write_default_config(home: &Path, text: &str) -> PathBuf {
    let path = test_env(Some(home)).config_file().expect("a config path");
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("config folder");
    std::fs::write(&path, text).expect("config file");
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch;

    const NO_HOME_KEY: &str = "invalid setting: access_key_file has no default without a home \
         directory: set HOME, or give access_key_file or no_access_key";

    #[test]
    fn without_a_home_the_database_is_an_empty_path() {
        let dir = scratch::Dir::new("settings-no-home-db");
        let config = dir.join("serve.toml");
        std::fs::write(&config, "no_access_key = true").expect("config");
        let settings =
            serve_settings(&test_env(None), Some(config), ServeOptions::NONE).expect("valid");
        assert_eq!(settings.db, PathBuf::new());
        assert_eq!(settings.access_key, AccessKey::None);
        assert_eq!(settings.bind, config::DEFAULT_BIND);
    }

    #[test]
    fn without_a_home_or_a_key_setting_the_key_has_no_default() {
        let env = test_env(None);
        assert_eq!(
            serve_settings(&env, None, ServeOptions::NONE).map(|s| s.db),
            Err(NO_HOME_KEY.to_string())
        );
        assert_eq!(serve_bind(&env), Err(NO_HOME_KEY.to_string()));
    }

    #[test]
    fn with_a_home_the_defaults_apply_and_flags_win_over_the_file() {
        let home = scratch::Dir::new("settings-home-defaults");
        let env = test_env(Some(&home));
        let settings = serve_settings(&env, None, ServeOptions::NONE).expect("valid");
        assert_eq!(Some(settings.db), env.default_db(Platform::CURRENT));
        assert_eq!(
            settings.access_key,
            AccessKey::Default(env.access_key_file().expect("key path"))
        );
        write_default_config(
            &home,
            "db = \"/data/file.db\"\nbind = \"127.0.0.1:8000\"\nmax_limit = 50",
        );
        let flags = ServeOptions {
            bind: Some("127.0.0.1:9000".parse().expect("addr")),
            ..ServeOptions::NONE
        };
        let settings = serve_settings(&env, None, flags).expect("valid");
        assert_eq!(settings.db, PathBuf::from("/data/file.db"));
        assert_eq!(settings.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(settings.limits.max_limit, 50);
        assert_eq!(
            serve_bind(&env).map(|b| b.to_string()),
            Ok("127.0.0.1:8000".into())
        );
    }

    #[test]
    fn a_bad_setting_names_the_config_file_only_when_it_set_it() {
        let home = scratch::Dir::new("settings-bad-setting");
        let env = test_env(Some(&home));
        let path = write_default_config(&home, "name_pack = \"missing\"");
        let err = serve_settings(&env, None, ServeOptions::NONE).expect_err("bad pack");
        assert!(
            err.starts_with("invalid setting: name_pack must be one of: "),
            "{err}"
        );
        assert!(
            err.ends_with(&format!(" (set in {})", path.display())),
            "{err}"
        );
        let flags = ServeOptions {
            name_pack: Some("also-missing".into()),
            ..ServeOptions::NONE
        };
        let err = serve_settings(&env, None, flags).expect_err("bad pack");
        assert!(!err.contains("(set in"), "{err}");
    }
}
