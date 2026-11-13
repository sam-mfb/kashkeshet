//! The machine's facts as this process sees them: [`env`] reads the
//! environment, and [`platform`] gives the platform this program was built
//! for, as kkt-machine's `Env` and `Platform`. The program asks once and
//! passes them on.

#[cfg(any(test, feature = "real"))]
use kkt_machine::{Env, Platform};
#[cfg(any(test, feature = "real"))]
use std::ffi::OsString;
#[cfg(any(test, feature = "real"))]
use std::path::PathBuf;

/// The environment as this process sees it: `$HOME` and the folders the
/// environment names.
#[cfg(any(test, feature = "real"))]
pub fn env() -> Env {
    env_from(|name| std::env::var_os(name))
}

/// The platform this program was built for. Only macOS and Linux are
/// released; every system but macOS follows XDG.
#[cfg(any(test, feature = "real"))]
pub fn platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Xdg
    }
}

/// The [`Env`] the variables `var` gives make: an unset or empty variable
/// is `None`.
#[cfg(any(test, feature = "real"))]
fn env_from(var: impl Fn(&str) -> Option<OsString>) -> Env {
    let path = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    Env {
        home: path("HOME"),
        claude_config_dir: path("CLAUDE_CONFIG_DIR"),
        codex_home: path("CODEX_HOME"),
        xdg_config_home: path("XDG_CONFIG_HOME"),
        xdg_data_home: path("XDG_DATA_HOME"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variable_fills_its_field_and_empty_ones_are_unset() {
        let env = env_from(|name| match name {
            "HOME" => Some("/home/sam".into()),
            "CLAUDE_CONFIG_DIR" => Some("".into()),
            "CODEX_HOME" => Some("/codex".into()),
            "XDG_CONFIG_HOME" => Some("relative".into()),
            _ => None,
        });
        assert_eq!(
            env,
            Env {
                home: Some(PathBuf::from("/home/sam")),
                claude_config_dir: None,
                codex_home: Some(PathBuf::from("/codex")),
                xdg_config_home: Some(PathBuf::from("relative")),
                xdg_data_home: None,
            }
        );
    }
}
