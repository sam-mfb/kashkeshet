//! Facts about the machine a program runs on: the folders its environment
//! names ([`Env`]) and its platform ([`Platform`]), as plain values, from
//! which callers build their own paths. [`Machine`] is the contract for
//! reading them for this process: a program asks once at start and passes
//! the values on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `$XDG_CONFIG_HOME` when it is not set, under the home directory.
pub const XDG_CONFIG_HOME_DEFAULT: &str = ".config";
/// `$XDG_DATA_HOME` when it is not set, under the home directory.
const XDG_DATA_HOME_DEFAULT: &str = ".local/share";

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
/// [`Env::data_home`], [`Env::absolute_xdg_config_home`] and
/// [`Env::absolute_xdg_data_home`].
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

    /// The base of per-user data under the XDG base directories:
    /// `$XDG_DATA_HOME`, or `~/.local/share`. `None` with neither. Where a
    /// platform keeps data elsewhere is the caller's choice.
    pub fn data_home(&self) -> Option<PathBuf> {
        match self.absolute_xdg_data_home() {
            Some(dir) => Some(dir.to_path_buf()),
            None => Some(self.home.as_ref()?.join(XDG_DATA_HOME_DEFAULT)),
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

/// The machine's facts as one process sees them. A program asks once, at
/// start, and passes the values down.
pub trait Machine {
    /// The environment's folders: `$HOME`, the XDG folders, and the
    /// folders the variables in `named` name ([`Env::named`]).
    fn env(&self, named: &[&str]) -> Env;
    /// The platform this program was built for.
    fn platform(&self) -> Platform;
    /// This process's ID, as the operating system numbers it.
    fn process_id(&self) -> u32;
    /// This program's absolute path, with links followed. The error is the
    /// operating system's.
    fn program_path(&self) -> std::io::Result<PathBuf>;
    /// The folders `$PATH` names, in order; none when it is unset.
    fn path_dirs(&self) -> Vec<PathBuf>;
    /// The folder this process works in, which relative paths are taken
    /// from. The error is the operating system's.
    fn current_dir(&self) -> std::io::Result<PathBuf>;
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stand_in_gives_only_the_named_folders_asked_for() {
        let mut given = env(None, None);
        given.named = [
            ("A_DIR".to_string(), PathBuf::from("/a")),
            ("B_DIR".to_string(), PathBuf::from("/b")),
        ]
        .into();
        let machine = testing::FixedMachine {
            env: given,
            platform: Platform::Xdg,
            process_id: 1,
            program_path: None,
            path_dirs: Vec::new(),
            current_dir: None,
        };
        let env = machine.env(&["A_DIR", "C_DIR"]);
        assert_eq!(env.named("A_DIR"), Some(Path::new("/a")));
        assert_eq!(env.named("B_DIR"), None);
        assert_eq!(env.named("C_DIR"), None);
    }

    #[test]
    fn the_stand_in_gives_the_facts_it_was_given() {
        let machine = testing::FixedMachine {
            env: env(Some("/config"), None),
            platform: Platform::MacOs,
            process_id: 42,
            program_path: Some(PathBuf::from("/bin/kkt")),
            path_dirs: vec![PathBuf::from("/bin")],
            current_dir: Some(PathBuf::from("/home/me")),
        };
        assert_eq!(machine.env(&[]), env(Some("/config"), None));
        assert_eq!(machine.platform(), Platform::MacOs);
        assert_eq!(machine.process_id(), 42);
        assert_eq!(
            machine.program_path().expect("a path"),
            PathBuf::from("/bin/kkt")
        );
        assert_eq!(machine.path_dirs(), [PathBuf::from("/bin")]);
        assert_eq!(
            machine.current_dir().expect("a folder"),
            PathBuf::from("/home/me")
        );
        let lost = testing::FixedMachine {
            program_path: None,
            current_dir: None,
            ..machine
        };
        assert_eq!(
            lost.program_path().expect_err("no path").kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            lost.current_dir().expect_err("no folder").kind(),
            std::io::ErrorKind::NotFound
        );
    }

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
    fn data_goes_under_xdg_data_home_else_dot_local_share() {
        assert_eq!(
            env(None, Some("/xdg/data")).data_home(),
            Some(PathBuf::from("/xdg/data"))
        );
        assert_eq!(
            env(None, Some("relative/data")).data_home(),
            Some(PathBuf::from("/home/sam/.local/share"))
        );
        assert_eq!(
            env(None, None).data_home(),
            Some(PathBuf::from("/home/sam/.local/share"))
        );
        let homeless = Env {
            home: None,
            ..env(None, None)
        };
        assert_eq!(homeless.data_home(), None);
        let homeless = Env {
            home: None,
            ..env(None, Some("/xdg/data"))
        };
        assert_eq!(homeless.data_home(), Some(PathBuf::from("/xdg/data")));
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
