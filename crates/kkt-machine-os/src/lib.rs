//! The machine's facts as this process sees them: [`env`] reads the
//! environment, [`platform`] gives the platform this program was built
//! for, as kkt-machine's `Env` and `Platform`, and [`process_id`] names this
//! process. The program asks once and passes them on.

use kkt_machine::{Env, Platform};
use std::ffi::OsString;
use std::path::PathBuf;

/// The environment as this process sees it: `$HOME`, the XDG folders, and
/// the folders the variables in `named` name.
pub fn env(named: &[&str]) -> Env {
    env_from(named, |name| std::env::var_os(name))
}

/// The platform this program was built for. Only macOS and Linux are
/// released; every system but macOS follows XDG.
pub fn platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Xdg
    }
}

/// This process's ID, as the operating system numbers it.
pub fn process_id() -> u32 {
    std::process::id()
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
