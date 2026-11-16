//! Facts about the machine kkt runs on: the folders its environment names
//! ([`Env`]) and its platform ([`Platform`]), as plain values. The crates
//! that place files on the machine take these facts and build their own
//! paths from them; none reads the environment itself. kkt-machine-os reads
//! them for this process, and the program passes them on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `$XDG_CONFIG_HOME` when it is not set, under the home directory.
pub const XDG_CONFIG_HOME_DEFAULT: &str = ".config";

/// The platform, which decides where per-user data goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    /// Linux and other Unix systems, which follow the XDG base directories.
    Xdg,
}

/// The environment values that decide default paths: the home and XDG
/// folders, and any other folders the program asked for by the variable
/// that names them ([`Env::named`]). Unset or empty values are absent. The
/// XDG values count only when absolute, as the XDG base directory
/// specification says: read them through [`Env::config_home`],
/// [`Env::absolute_xdg_config_home`] and [`Env::absolute_xdg_data_home`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Env {
    /// `$HOME`.
    pub home: Option<PathBuf>,
    /// The other folders asked for, by the name of the variable that names
    /// each, such as `PROGRAM_CONFIG_DIR`. Only the set ones are here.
    pub named: BTreeMap<String, PathBuf>,
    /// `$XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<PathBuf>,
    /// `$XDG_DATA_HOME`.
    pub xdg_data_home: Option<PathBuf>,
}

impl Env {
    /// The folder the environment variable `variable` names, if it was
    /// asked for and is set.
    pub fn named(&self, variable: &str) -> Option<&Path> {
        self.named.get(variable).map(PathBuf::as_path)
    }

    /// The base of per-user config: `$XDG_CONFIG_HOME`, or `~/.config`. The
    /// same on every platform, and shared by other programs' config, such as
    /// systemd user units. `None` with neither.
    pub fn config_home(&self) -> Option<PathBuf> {
        match self.absolute_xdg_config_home() {
            Some(dir) => Some(dir.to_path_buf()),
            None => Some(self.home.as_ref()?.join(XDG_CONFIG_HOME_DEFAULT)),
        }
    }

    /// `$XDG_CONFIG_HOME` when it is absolute; otherwise `None`.
    pub fn absolute_xdg_config_home(&self) -> Option<&Path> {
        absolute(&self.xdg_config_home)
    }

    /// `$XDG_DATA_HOME` when it is absolute; otherwise `None`.
    pub fn absolute_xdg_data_home(&self) -> Option<&Path> {
        absolute(&self.xdg_data_home)
    }
}

fn absolute(path: &Option<PathBuf>) -> Option<&Path> {
    path.as_deref().filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(config: Option<&str>, data: Option<&str>) -> Env {
        Env {
            home: Some(PathBuf::from("/home/sam")),
            named: BTreeMap::new(),
            xdg_config_home: config.map(PathBuf::from),
            xdg_data_home: data.map(PathBuf::from),
        }
    }

    #[test]
    fn absolute_xdg_values_count() {
        let e = env(Some("/xdg/config"), Some("/xdg/data"));
        assert_eq!(e.absolute_xdg_config_home(), Some(Path::new("/xdg/config")));
        assert_eq!(e.absolute_xdg_data_home(), Some(Path::new("/xdg/data")));
    }

    #[test]
    fn relative_or_unset_xdg_values_do_not() {
        let e = env(Some("relative/config"), Some("relative/data"));
        assert_eq!(e.absolute_xdg_config_home(), None);
        assert_eq!(e.absolute_xdg_data_home(), None);
        let e = env(None, None);
        assert_eq!(e.absolute_xdg_config_home(), None);
        assert_eq!(e.absolute_xdg_data_home(), None);
    }

    #[test]
    fn config_goes_under_xdg_config_home_else_dot_config() {
        assert_eq!(
            env(Some("/xdg/config"), None).config_home(),
            Some(PathBuf::from("/xdg/config"))
        );
        assert_eq!(
            env(Some("relative/config"), None).config_home(),
            Some(PathBuf::from("/home/sam/.config"))
        );
        let homeless = Env {
            home: None,
            ..env(None, None)
        };
        assert_eq!(homeless.config_home(), None);
        let homeless = Env {
            home: None,
            ..env(Some("/xdg/config"), None)
        };
        assert_eq!(homeless.config_home(), Some(PathBuf::from("/xdg/config")));
    }
}
