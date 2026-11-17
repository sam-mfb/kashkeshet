//! The machine's facts as this process sees them: [`OsMachine`] implements
//! [`Machine`](kkt_machine::Machine), reading the environment, naming the platform
//! this program was built for, and giving this process's ID, this
//! program's path and the folders `$PATH` names. The program
//! asks once and passes the values on.

use kkt_machine::{Env, Machine, Platform};
use std::ffi::OsString;
use std::path::PathBuf;

/// This process on this machine.
pub struct OsMachine;

impl Machine for OsMachine {
    /// The environment as this process sees it: `$HOME`, the XDG folders,
    /// and the folders the variables in `named` name.
    fn env(&self, named: &[&str]) -> Env {
        env_from(named, |name| std::env::var_os(name))
    }

    /// The platform this program was built for. Only macOS and Linux are
    /// released; every system but macOS follows XDG.
    fn platform(&self) -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Xdg
        }
    }

    fn process_id(&self) -> u32 {
        std::process::id()
    }

    fn program_path(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe().and_then(|p| p.canonicalize())
    }

    fn path_dirs(&self) -> Vec<PathBuf> {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default()
    }
}

/// The [`Env`] the variables `var` gives make, with the folders `named`
/// asks for: an unset or empty variable is left out.
fn env_from(named: &[&str], var: impl Fn(&str) -> Option<OsString>) -> Env {
    let path = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    Env {
        home: path("HOME"),
        named: named
            .iter()
            .filter_map(|name| path(name).map(|dir| (name.to_string(), dir)))
            .collect(),
        xdg_config_home: path("XDG_CONFIG_HOME"),
        xdg_data_home: path("XDG_DATA_HOME"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variable_fills_its_field_and_empty_ones_are_unset() {
        let env = env_from(&["CLAUDE_CONFIG_DIR", "CODEX_HOME"], |name| match name {
            "HOME" => Some("/home/sam".into()),
            "CLAUDE_CONFIG_DIR" => Some("".into()),
            "CODEX_HOME" => Some("/codex".into()),
            "XDG_CONFIG_HOME" => Some("relative".into()),
            // Set, but not asked for: left out.
            "OTHER_DIR" => Some("/other".into()),
            _ => None,
        });
        assert_eq!(
            env,
            Env {
                home: Some(PathBuf::from("/home/sam")),
                named: [("CODEX_HOME".to_string(), PathBuf::from("/codex"))].into(),
                xdg_config_home: Some(PathBuf::from("relative")),
                xdg_data_home: None,
            }
        );
    }
}
