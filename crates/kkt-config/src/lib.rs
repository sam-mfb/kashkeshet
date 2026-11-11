//! Server limits and fixed settings. Every default is written out here, in
//! one place, along with the checks a server's settings must pass at startup.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

// ---- Fixed settings ----
//
// Values agents never see and the command line does not change.

/// The longest the system clock's sleeps go without looking at the wall
/// clock. Timers stop while the computer sleeps, so a sleep is a series of
/// timers this long, each followed by a look at the time; one long timer
/// could end long after its deadline.
pub const CLOCK_RECHECK: Duration = Duration::from_secs(1);
/// Unix permissions of a backup's folder, which holds a copy of the access
/// key: only the owner may list or enter it.
pub const BACKUP_FOLDER_MODE: u32 = 0o700;
/// The folder, beside the database file, where the server keeps the backups
/// of the database it makes before upgrading it.
pub const UPGRADE_BACKUP_FOLDER: &str = "upgrade-backups";
/// The Compose project name for kkt in Docker, so the containers and volumes
/// are named the same whatever the folder holding the Compose files is called.
pub const DOCKER_PROJECT: &str = "kkt";
/// The folder, in the per-user data directory, that `kkt setup` keeps
/// the Docker install's files in.
pub const DOCKER_FOLDER: &str = "docker";
/// Where `kkt setup` saves the Docker server's access key, in the config
/// directory.
pub const DOCKER_KEY_FILE_NAME: &str = "docker-access-key";
/// The machine name `kkt setup` asks for on the tailnet.
pub const DEFAULT_TS_HOSTNAME: &str = "kkt";
/// How long `kkt setup` waits for a server it just set up to answer before
/// its connection check gives up. A new Tailscale node gets its HTTPS
/// certificate on the first request, which can take a while.
pub const SETUP_SERVER_WAIT: Duration = Duration::from_secs(2 * 60);
/// How often `kkt setup` asks while it waits.
pub const SETUP_SERVER_POLL: Duration = Duration::from_secs(2);
/// The user ID the kkt image runs as. A host folder it writes backups to
/// must belong to it.
pub const DOCKER_IMAGE_USER_ID: u32 = 10001;
/// The folder inside the managed Docker folder that is mounted at
/// [`DOCKER_BACKUPS_MOUNT`] in the container.
pub const DOCKER_BACKUPS_DIR: &str = "backups";
/// Where the Compose file mounts the backups folder in the container.
pub const DOCKER_BACKUPS_MOUNT: &str = "/backups";
/// The start of the name of the folder in the kkt container that `kkt lanes
/// import` copies the lane files into, to import them there. Each run makes
/// its own with `mktemp -d`, which adds random characters, and removes only
/// that one afterwards, so runs at the same time do not share a folder.
pub const DOCKER_LANES_IMPORT_DIR: &str = "/tmp/kkt-lanes-import";
/// The start of the name of the folder in the kkt container that `kkt lanes
/// export` writes the lane files to, to copy them out; one per run, as for
/// [`DOCKER_LANES_IMPORT_DIR`].
pub const DOCKER_LANES_EXPORT_DIR: &str = "/tmp/kkt-lanes-export";
/// The environment variables Compose reads, which would take the place of
/// the managed folder's settings file if they were set where kkt runs, so
/// every Docker command runs without them.
pub const DOCKER_MANAGED_VARIABLES: &[&str] = &[
    "COMPOSE_FILE",
    "COMPOSE_PATH_SEPARATOR",
    "COMPOSE_PROFILES",
    "COMPOSE_PROJECT_NAME",
    "COMPOSE_ENV_FILES",
    "KKT_VERSION",
    "KKT_PORT",
    "TS_HOSTNAME",
    "TS_TAILNET",
    "TS_AUTHKEY",
];
/// How long to wait for someone to log the Tailscale container in.
pub const DOCKER_LOGIN_WAIT: Duration = Duration::from_secs(10 * 60);
/// How long to wait for the kkt container to start and make its access key.
pub const DOCKER_START_WAIT: Duration = Duration::from_secs(2 * 60);
/// How many checks to wait for a login link from a Tailscale container that
/// needs a login before restarting it. A node whose key expired, or that was
/// logged out, is not asking for a login, so it shows no link until it is
/// restarted; a new one shows its link within a few seconds.
pub const DOCKER_LINK_WAIT_POLLS: u32 = 5;
/// How often to ask while waiting for the Tailscale login or for kkt.
pub const DOCKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// What `TS_TAILNET` is set to until the Tailscale container has logged in
/// and the real name is known. The Compose file needs some value to start
/// the Tailscale container alone; kkt does not run with it.
pub const DOCKER_PENDING_TAILNET: &str = "pending.invalid";

// ---- Server defaults and where files live ----
//
// What a server uses when neither a flag nor the config file says otherwise.

/// Address to listen on.
pub const DEFAULT_BIND: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7420));
/// Whether `bind` may be a non-loopback address.
pub const DEFAULT_ALLOW_ANY_BIND: bool = false;
/// Browser origins allowed to call `/mcp` (`allow_origins`): none.
pub const DEFAULT_ALLOW_ORIGINS: &[&str] = &[];
/// Host names the server answers to besides loopback (`allow_hosts`): none.
pub const DEFAULT_ALLOW_HOSTS: &[&str] = &[];
/// Database file name in the per-user data directory, for `kkt serve`.
pub const DB_FILE_NAME: &str = "kashkeshet.db";
/// The per-user data directory on macOS, under the home directory.
pub const MACOS_DATA_DIR: &str = "Library/Application Support/kashkeshet";
/// The per-user data directory elsewhere, under `$XDG_DATA_HOME`.
pub const XDG_DATA_DIR: &str = "kashkeshet";
/// `$XDG_DATA_HOME` when it is not set, under the home directory.
pub const XDG_DATA_HOME_DEFAULT: &str = ".local/share";
/// The config directory, under `$XDG_CONFIG_HOME`, on every platform.
pub const CONFIG_DIR: &str = "kkt";
/// `$XDG_CONFIG_HOME` when it is not set, under the home directory.
pub const XDG_CONFIG_HOME_DEFAULT: &str = ".config";
/// The config file `kkt serve` reads, in the config directory, if it exists.
pub const CONFIG_FILE_NAME: &str = "config.toml";
/// Docker setup's host copy of the operator key.
pub const DOCKER_WEB_KEY_FILE_NAME: &str = "docker-web-key";
/// Docker setup's host copy of the admin key.
pub const DOCKER_ADMIN_KEY_FILE_NAME: &str = "docker-admin-key";
/// The admin name `kkt setup` gives a new admin key when not asked, at a
/// terminal (where it is offered) or without one. It becomes
/// `admin-supervisor`.
pub const DEFAULT_SETUP_ADMIN_NAME: &str = "supervisor";
/// The exit code of `kkt key show --admin` when there is no admin key.
pub const NO_ADMIN_KEY_EXIT: u8 = 4;
/// The exit code of a command given an option it does not know, as a kkt
/// from before admin keys is given `kkt key show --admin`.
pub const UNKNOWN_OPTION_EXIT: i32 = 2;
/// Whether `kkt setup` makes an admin key when it is not asked, at a
/// terminal or without one.
pub const DEFAULT_SETUP_ADMIN_KEY: bool = false;
/// The setup record `kkt setup` writes, in the config directory.
pub const SETUP_FILE_NAME: &str = "setup.toml";
/// Where `kkt setup` notes a server it installed but did not record, in the
/// config directory.
pub const SETUP_UNFINISHED_FILE_NAME: &str = "setup-unfinished.toml";
/// The `written_by` of a record `kkt setup` wrote for a service it found
/// installed with no record, whose version nothing says. `kkt upgrade`
/// always installs such a service again.
pub const SETUP_UNKNOWN_VERSION: &str = "unknown";
/// The zero-based mode option Enter accepts during interactive setup (Docker).
pub const DEFAULT_SETUP_MODE_INDEX: usize = 0;
/// Whether Enter allows agents in local containers during interactive setup.
pub const DEFAULT_SETUP_CONTAINERS: bool = true;
/// The name Docker gives containers for the machine they run on, which a
/// server set up for agents in local containers also answers to.
pub const SETUP_DOCKER_HOST: &str = "host.docker.internal";
/// Whether Enter tries a failed step again during interactive setup.
pub const DEFAULT_SETUP_TRY_AGAIN: bool = true;
/// Whether Enter continues an unfinished setup during interactive setup.
pub const DEFAULT_SETUP_CONTINUE: bool = true;
/// Whether Enter connects a client found during interactive setup.
pub const DEFAULT_SETUP_CONNECT_CLIENT: bool = true;
/// Whether `kkt setup` lets the clients it connects always use kkt's tools
/// and `kkt watch` when it is not asked, at a terminal or without one.
pub const DEFAULT_SETUP_ALLOW_KKT: bool = false;
/// Whether Enter shares the server over Tailscale during interactive setup.
pub const DEFAULT_SETUP_TAILSCALE: bool = false;
/// Whether Enter records a found, unrecorded install during interactive
/// setup.
pub const DEFAULT_SETUP_ADOPT: bool = true;
/// The client-only server address Enter accepts during interactive setup.
pub const DEFAULT_SETUP_URL: &str = "http://127.0.0.1:7420/mcp";
/// The client-only access key Enter accepts (no key).
pub const DEFAULT_SETUP_KEY: &str = "";
/// Where `kkt setup` keeps a client-only machine's copy of its server's
/// key, in the config directory.
pub const CLIENT_KEY_FILE_NAME: &str = "client-access-key";
/// The host Tailscale Serve listener used by service-mode setup.
pub const TAILSCALE_HTTPS_PORT: u16 = 443;
/// Full backups made before an explicit service-mode upgrade, in the
/// user's kkt config directory.
pub const SERVICE_BACKUP_FOLDER: &str = "backups";

// Client setup and diagnostics.
/// `$CODEX_HOME` when it is not set, under the home directory.
pub const CODEX_HOME_DEFAULT: &str = ".codex";
/// Codex's config file, in `$CODEX_HOME`.
pub const CODEX_CONFIG: &str = "config.toml";
pub const CLAUDE_CONFIG: &str = ".claude.json";
/// Claude Code's settings folder when `$CLAUDE_CONFIG_DIR` is not set,
/// under the home directory.
pub const CLAUDE_HOME_DEFAULT: &str = ".claude";
/// Claude Code's user settings, with its permission rules, in
/// `$CLAUDE_CONFIG_DIR` or [`CLAUDE_HOME_DEFAULT`].
pub const CLAUDE_SETTINGS: &str = "settings.json";
/// Codex's folder of command rules, in `$CODEX_HOME`.
pub const CODEX_RULES_DIR: &str = "rules";
/// The rules file `kkt setup` writes in [`CODEX_RULES_DIR`].
pub const CODEX_KKT_RULES: &str = "kkt.rules";
pub const AGY_CONFIG: &str = ".gemini/config/mcp_config.json";
pub const AGY_LEGACY_CONFIG: &str = ".gemini/antigravity/mcp_config.json";
/// The Antigravity CLI's settings, with its permission rules, under the
/// home directory.
pub const AGY_CLI_SETTINGS: &str = ".gemini/antigravity-cli/settings.json";
/// OpenCode's config file, under `$XDG_CONFIG_HOME`.
pub const OPENCODE_CONFIG: &str = "opencode/opencode.json";
/// Muse Code's settings folder, under the config home (`$XDG_CONFIG_HOME`,
/// or `~/.config`).
pub const MUSE_CONFIG_DIR: &str = "muse";
/// Muse Code's user settings file, in [`MUSE_CONFIG_DIR`], so
/// `~/.config/muse/settings.json` by default.
pub const MUSE_CONFIG: &str = "settings.json";
/// The programs `kkt setup` looks for on PATH to tell which clients are
/// installed.
pub const CLAUDE_PROGRAM: &str = "claude";
pub const CODEX_PROGRAM: &str = "codex";
pub const OPENCODE_PROGRAM: &str = "opencode";
pub const MUSE_PROGRAM: &str = "muse";
pub const DOCTOR_TIMEOUT: Duration = Duration::from_secs(5);
pub const DOCTOR_MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const DOCTOR_PROTOCOL_VERSION: &str = "2026-07-28";

// ---- `kkt watch` ----

/// The environment variable `kkt watch` reads the agent's token from when
/// no `--token-file` is given.
pub const WATCH_TOKEN_ENV: &str = "KKT_TOKEN";
/// How long each `wait_for_messages` call asks to wait. The server caps it
/// at its own `max_wait`.
pub const WATCH_WAIT: Duration = Duration::from_secs(50);
/// How long past its wait a call may take before it counts as lost.
pub const WATCH_CALL_GRACE: Duration = Duration::from_secs(30);
/// How many messages each call asks for.
pub const WATCH_BATCH: u32 = 100;
/// How often `kkt watch --admin` asks the admin endpoint, which does not
/// wait, for new messages while none come.
pub const WATCH_ADMIN_POLL: Duration = Duration::from_secs(2);
/// The name `kkt watch --admin` watches as. No agent can have it: agent
/// names have no parentheses.
pub const WATCH_ADMIN_NAME: &str = "(admin)";
/// The first pause after a lost connection; it doubles up to the maximum.
pub const WATCH_RETRY_FIRST: Duration = Duration::from_secs(2);
pub const WATCH_RETRY_MAX: Duration = Duration::from_secs(60);
/// The most characters of a message body on its line.
pub const WATCH_BODY_CHARS: usize = 300;
/// The exit code of `kkt watch --once --timeout` when no message came.
pub const WATCH_TIMED_OUT_EXIT: u8 = 3;

// ---- Running as a service ----
//
// What `kkt service` writes and runs. See `service`.

/// The launchd job label on macOS. The plist is `<label>.plist`.
pub const LAUNCHD_LABEL: &str = "com.sam-mfb.kashkeshet";
/// Where launchd user agents live, under the home directory.
pub const LAUNCH_AGENTS_DIR: &str = "Library/LaunchAgents";
/// Where the service's log goes on macOS, under the home directory. Linux
/// services log to the journal.
pub const MACOS_LOG_DIR: &str = "Library/Logs/kashkeshet";
/// The service's log file on macOS, in [`MACOS_LOG_DIR`]. The server writes
/// it itself (`--log-file`), within [`LogPolicy::DEFAULT`].
pub const MACOS_LOG_FILE_NAME: &str = "kashkeshet.log";
/// Where launchd sends the service's stdout and stderr on macOS, in
/// [`MACOS_LOG_DIR`]. Only output from before logging starts, and crashes,
/// land here.
pub const MACOS_CRASH_LOG_FILE_NAME: &str = "kashkeshet.crash.log";
/// The systemd user unit on Linux.
pub const SYSTEMD_UNIT: &str = "kashkeshet.service";
/// Where systemd user units live, under `$XDG_CONFIG_HOME`.
pub const SYSTEMD_USER_DIR: &str = "systemd/user";
/// How long the service manager waits before starting the server again
/// after it failed.
pub const SERVICE_RESTART_DELAY: Duration = Duration::from_secs(5);
/// How long `kkt service install` waits after starting the service before
/// it checks that the server is still running. A server that fails as it
/// starts, such as when another program has its port, has exited by then.
/// It is shorter than [`SERVICE_RESTART_DELAY`], so the service manager has
/// not yet started such a server again when the check is made.
pub const SERVICE_START_CHECK: Duration = Duration::from_secs(3);
const _: () = assert!(SERVICE_START_CHECK.as_millis() < SERVICE_RESTART_DELAY.as_millis());
/// How long the service manager waits for the server to stop before it
/// kills it.
pub const SERVICE_STOP_TIMEOUT: Duration = Duration::from_secs(30);
/// How long `kkt service install` waits for launchd to remove the old
/// service after unloading it, trying to load the new one meanwhile.
/// `launchctl bootout` returns before launchd has finished removing the
/// service, and a `bootstrap` before it has fails ("Operation already in
/// progress", which launchctl shows as "Input/output error"). The old
/// server has [`SERVICE_STOP_TIMEOUT`] to exit; the rest is a margin for
/// launchd's own work after that.
pub const LAUNCHD_REMOVAL_WAIT: Duration = Duration::from_secs(SERVICE_STOP_TIMEOUT.as_secs() + 5);
/// How long to wait between those tries.
pub const LAUNCHD_RELOAD_RETRY: Duration = Duration::from_millis(250);
/// Log lines `kkt service logs` shows when not told how many.
pub const DEFAULT_LOG_LINES: usize = 100;

// ---- Logging ----

/// How much is logged: the named level and everything more severe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Every level, least verbose first, as named in flags and the config file.
    pub const ALL: [(LogLevel, &'static str); 5] = [
        (LogLevel::Error, "error"),
        (LogLevel::Warn, "warn"),
        (LogLevel::Info, "info"),
        (LogLevel::Debug, "debug"),
        (LogLevel::Trace, "trace"),
    ];

    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }

    /// The level named `text`, or an error naming `setting` and the choices.
    pub fn parse(setting: &'static str, text: &str) -> Result<LogLevel, InvalidSetting> {
        LogLevel::ALL
            .iter()
            .find(|(_, name)| *name == text)
            .map(|(level, _)| *level)
            .ok_or_else(|| {
                InvalidSetting::new(
                    setting,
                    format!("must be one of error, warn, info, debug, trace, not {text:?}"),
                )
            })
    }
}

/// The level logged for kkt's own code (`log_level`).
pub const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Info;
/// The level logged for the libraries kkt uses (`dependency_log_level`).
/// Above info, because the MCP library logs several lines for every request.
pub const DEFAULT_DEPENDENCY_LOG_LEVEL: LogLevel = LogLevel::Warn;

/// Checks `dependency_log_level`. At debug and trace the MCP library logs
/// whole requests and replies, tokens and recovery codes included, and logs
/// must never hold those; so those levels are refused.
pub fn check_dependency_log_level(level: LogLevel) -> Result<(), InvalidSetting> {
    match level {
        LogLevel::Error | LogLevel::Warn | LogLevel::Info => Ok(()),
        LogLevel::Debug | LogLevel::Trace => Err(InvalidSetting::new(
            "dependency_log_level",
            format!(
                "must be error, warn or info, not {}: at debug and trace the MCP library \
                 logs request contents, including tokens",
                level.name()
            ),
        )),
    }
}
/// The targets of kkt's own code: the library and the `kkt` binary.
pub const LOG_OWN_TARGETS: &[&str] = &["kashkeshet", "kkt"];
/// How often a log file the server writes itself looks for rolled files past
/// their age, besides when it opens and when it rolls over. It looks on the
/// first write after this much time, so a server that logs nothing deletes
/// nothing until it next logs.
pub const LOG_PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Smallest allowed [`LogPolicy::max_bytes`].
pub const MIN_LOG_FILE_BYTES: u64 = 64 * 1024;

/// How a log file the server writes itself (`--log-file`) is kept bounded.
/// When the file would grow past `max_bytes`, it is renamed `<file>.1`, the
/// older ones move up to `<file>.<rotated_files>`, and the oldest is
/// deleted. Rolled files older than `max_age` are deleted too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogPolicy {
    pub max_bytes: u64,
    pub rotated_files: usize,
    pub max_age: Duration,
}

impl LogPolicy {
    /// At most about 60 MB: the current file and five rolled ones of 10 MB,
    /// none rolled more than 30 days ago.
    pub const DEFAULT: LogPolicy = LogPolicy {
        max_bytes: 10 * 1024 * 1024,
        rotated_files: 5,
        max_age: Duration::from_secs(30 * 24 * 60 * 60),
    };

    /// Checks that the policy bounds the log. The server refuses to start
    /// with one that fails.
    pub fn check(&self) -> Result<(), InvalidSetting> {
        if self.max_bytes < MIN_LOG_FILE_BYTES {
            return Err(InvalidSetting::new(
                "log_max_bytes",
                format!("must be at least {MIN_LOG_FILE_BYTES}"),
            ));
        }
        if self.rotated_files == 0 {
            return Err(InvalidSetting::new(
                "log_rotated_files",
                "must be at least 1",
            ));
        }
        if self.max_age < Duration::from_secs(60 * 60) {
            return Err(InvalidSetting::new(
                "log_max_age",
                "must be at least an hour",
            ));
        }
        Ok(())
    }
}

/// A setting the server refuses to start with.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub struct InvalidSetting {
    pub name: &'static str,
    pub rule: String,
}

impl InvalidSetting {
    pub fn new(name: &'static str, rule: impl Into<String>) -> InvalidSetting {
        InvalidSetting {
            name,
            rule: rule.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_log_policy_passes_and_bad_ones_are_named() {
        assert_eq!(LogPolicy::DEFAULT.check(), Ok(()));
        let cases = [
            (
                LogPolicy {
                    max_bytes: MIN_LOG_FILE_BYTES - 1,
                    ..LogPolicy::DEFAULT
                },
                "log_max_bytes",
            ),
            (
                LogPolicy {
                    rotated_files: 0,
                    ..LogPolicy::DEFAULT
                },
                "log_rotated_files",
            ),
            (
                LogPolicy {
                    max_age: Duration::from_secs(59 * 60),
                    ..LogPolicy::DEFAULT
                },
                "log_max_age",
            ),
        ];
        for (policy, name) in cases {
            assert_eq!(policy.check().map_err(|e| e.name), Err(name));
        }
    }

    #[test]
    fn log_levels_parse_by_name_and_refuse_others() {
        for (level, name) in LogLevel::ALL {
            assert_eq!(LogLevel::parse("log_level", name), Ok(level));
            assert_eq!(level.name(), name);
        }
        let err = LogLevel::parse("dependency_log_level", "verbose").expect_err("unknown");
        assert_eq!(err.name, "dependency_log_level");
        assert!(LogLevel::parse("log_level", "INFO").is_err());
    }

    #[test]
    fn dependency_levels_that_would_log_secrets_are_refused() {
        for level in [LogLevel::Error, LogLevel::Warn, LogLevel::Info] {
            assert_eq!(check_dependency_log_level(level), Ok(()));
        }
        for level in [LogLevel::Debug, LogLevel::Trace] {
            let err = check_dependency_log_level(level).expect_err("refused");
            assert_eq!(err.name, "dependency_log_level");
        }
    }
}
