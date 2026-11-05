//! This process: its environment, what a closed standard output does to
//! it, its program's path, and writing a file whole.

use kkt_settings::Env;
use std::path::{Path, PathBuf};

/// Lets a closed standard output, as with `kkt status | head`, end the
/// command the way it ends other Unix commands (SIGPIPE), where Rust would
/// otherwise panic at the next print. Not for `kkt serve`: a server keeps
/// running when whatever reads its output goes away, and systemd starts it
/// with SIGPIPE ignored on purpose. Network writes are unaffected: Rust
/// asks the system not to signal for those.
pub fn end_on_closed_output() {
    // SAFETY: this sets only what happens on SIGPIPE, to the system's
    // default; no handler of ours runs, and nothing else in kkt sets it.
    #[cfg(unix)]
    if unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) } == libc::SIG_ERR {
        eprintln!(
            "warning: could not restore the default for a closed output: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// The environment values that decide default paths.
pub fn process_env() -> Env {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    Env {
        home: var("HOME"),
        claude_config_dir: var("CLAUDE_CONFIG_DIR"),
        codex_home: var("CODEX_HOME"),
        xdg_config_home: var("XDG_CONFIG_HOME"),
        xdg_data_home: var("XDG_DATA_HOME"),
    }
}

/// This program's absolute path, with links followed: the one place it is
/// found. Each caller says in its own words why it could not be.
pub fn program_path() -> std::io::Result<PathBuf> {
    std::env::current_exe().and_then(|p| p.canonicalize())
}

/// This program's absolute path, for client configs that run it.
pub fn kkt_path() -> Result<PathBuf, String> {
    program_path().map_err(|e| format!("could not find kkt's path: {e}"))
}

/// Writes `text` to `path` whole, then renames it into place, so a failure
/// leaves the earlier file.
pub fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let partial = path.with_extension("toml.partial");
    std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}
