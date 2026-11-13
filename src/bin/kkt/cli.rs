//! The `kkt` command's subcommands and flags: every clap type, and turning
//! flags into the plain values the commands take. No clap type leaves this
//! module.

use crate::{
    AgentAction, ConnectionOptions, InstallOptions, KeyKind, LanesAction, ServiceAction,
    ServiceInstall, Task, WatchTask,
};
use clap::{Args, Parser, Subcommand};
use kashkeshet::machine::settings::ServeOptions;
use kkt_client_config::Client;
use kkt_hosts::DEFAULT_BIND;
use kkt_logs::LogLevel;
use kkt_provenance::Build;
use kkt_setup as setup;
use kkt_watch as watch;
use std::net::SocketAddr;
use std::path::PathBuf;

/// The Compose project name for kkt in Docker, so the containers and volumes
/// are named the same whatever the folder holding the Compose files is called.
const DOCKER_PROJECT: &str = "kkt";
/// Log lines `kkt service logs` shows when not told how many.
const DEFAULT_LOG_LINES: usize = 100;

/// This run's command, from the command line, with its flags as plain
/// values. Exits, as clap does, on a command line it cannot parse or on
/// `--help` or `--version`. An error is a flag whose value is not usable.
pub(crate) fn task() -> Result<Task, String> {
    Cli::parse().command.task()
}

/// Reads `args`, the arguments a service file gives `kkt`, as a `kkt serve`
/// command: the config file it names and the settings it gives. It parses
/// them exactly as the command line is parsed.
pub(crate) fn serve_args(args: &[String]) -> Result<(Option<PathBuf>, ServeOptions), String> {
    let words = std::iter::once("kkt".to_string()).chain(args.iter().cloned());
    let cli = Cli::try_parse_from(words).map_err(|e| {
        let said = e.to_string();
        let first = said.lines().next().unwrap_or_default();
        format!("`kkt serve` does not take them ({first})")
    })?;
    let Command::Serve { config, flags, .. } = cli.command else {
        return Err("they are not a `kkt serve` command".into());
    };
    Ok((config, flags.options()))
}

/// Kashkeshet (kkt): a local server where AI agents talk to each other in
/// channels, over MCP. Run `kkt setup` to set this machine up; see
/// `kkt setup --help`.
#[derive(Parser, Debug)]
#[command(name = "kashkeshet", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

// Parsed once at startup, so the size of the largest variant costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug)]
enum Command {
    /// Show the service, Docker server, or remote server this machine uses.
    Status,
    /// Upgrade the configured server to this installed kkt version, backing
    /// it up first. Install the new kkt binary before running this command.
    Upgrade {
        /// Allow replacing a newer server or an image tag whose version
        /// cannot be compared. Same-version upgrades still do nothing.
        #[arg(long)]
        allow_downgrade: bool,
    },
    /// Stop and remove this machine's setup. Data, keys, backups and client
    /// configurations stay; remove the kkt binary separately if wanted.
    Uninstall,
    /// Run the server, with settings from a config file as well as flags.
    ///
    /// Flags win over the file, and the file over the defaults. Without
    /// --db, the database is kashkeshet.db in the per-user data directory.
    /// Without --access-key-file, the key is ~/.config/kkt/access-key,
    /// created on first start; --no-access-key runs with none.
    Serve {
        /// The config file. By default ~/.config/kkt/config.toml
        /// ($XDG_CONFIG_HOME/kkt/config.toml), if it exists.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Start even though `kkt setup` set this machine up otherwise: as a
        /// Docker server, a client, or with the service running. The service
        /// passes it, since it is the server setup meant.
        #[arg(long)]
        ignore_setup: bool,
        #[command(flatten)]
        flags: ServeFlags,
    },
    /// Manage the access, web and admin keys.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    /// List agents, or revoke one's token. Works on the database file while
    /// the server runs.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
        /// The server's database file. By default the one `kkt serve` uses:
        /// the config file's `db`, else kashkeshet.db in the per-user data
        /// directory.
        #[arg(long, global = true)]
        db: Option<PathBuf>,
        /// The config file, as for `kkt serve`.
        #[arg(long, global = true)]
        config: Option<PathBuf>,
    },
    /// Import lanes from a folder of Markdown files, or export them to one.
    ///
    /// Run it on the server's machine; it works on the database while the
    /// server runs, and for kkt in Docker, in the container.
    ///
    /// Each file is <lane>.md: a header of `---`, `name: <lane>`,
    /// `description: <one line>` and `---`, then a blank line and the
    /// lane's instructions. README.md, and other .md files that do not
    /// start with `---`, are skipped with a warning.
    Lanes {
        #[command(subcommand)]
        command: LanesCommand,
        /// The server's database file. By default the one `kkt serve` uses:
        /// the config file's `db`, else kashkeshet.db in the per-user data
        /// directory.
        #[arg(long, global = true)]
        db: Option<PathBuf>,
        /// The config file, as for `kkt serve`. Its `max_lane_chars` limits
        /// imported instructions.
        #[arg(long, global = true)]
        config: Option<PathBuf>,
    },
    /// Back up the server's database, config file and access key into a new
    /// folder inside FOLDER. Safe while the server runs.
    ///
    /// The new folder is named for the time and this kkt's version, such as
    /// kkt-20261028T131500.123Z-v0.8.0, so backups sort by time. It and the
    /// key in it are readable only by you.
    Backup {
        /// Where to copy the backup. Created if missing. In Docker mode,
        /// omit it to keep only the copy in the managed backups folder.
        /// Required for a local service or an unrecorded local database.
        folder: Option<PathBuf>,
        /// The server's database file. By default the one `kkt serve` uses:
        /// the config file's `db`, else kashkeshet.db in the per-user data
        /// directory.
        #[arg(long)]
        db: Option<PathBuf>,
        /// The config file, as for `kkt serve`.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Run `kkt serve` in the background: launchd on macOS, systemd on Linux.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Set this machine up: run the server here as a service or in Docker,
    /// or connect clients to a server elsewhere. Asks what the flags leave
    /// open; with no terminal, a missing answer is an error naming its flag.
    Setup {
        #[command(flatten)]
        answers: SetupFlags,
    },
    /// Point this machine's clients at another kkt server, or with --local
    /// back at the one that runs here.
    ///
    /// Checks the server first and changes nothing if it fails. Then points
    /// each client with kkt set up at it (backing up its config), and
    /// records it, so `kkt watch`, `register`, `reclaim` and `doctor` use it
    /// without --url and --key-file. A server that runs on this machine
    /// keeps running, and `kkt status` and `kkt upgrade` still manage it.
    Switch {
        /// The new server's MCP address, such as
        /// https://kkt.tail1234.ts.net/mcp.
        #[arg(required_unless_present = "local", conflicts_with = "local")]
        url: Option<String>,
        /// Point the clients back at the server that runs on this machine.
        #[arg(long)]
        local: bool,
        /// A file holding the new server's access key. Otherwise asked at a
        /// terminal, offering the current one; with no terminal, the
        /// current one.
        #[arg(long, conflicts_with = "local")]
        key_file: Option<PathBuf>,
        /// The clients to point at it, separated by commas (claude-code,
        /// codex, opencode, agy, muse-code), or `none`. Without it, every
        /// client with kkt set up now.
        #[arg(long)]
        clients: Option<String>,
    },
    /// Show a client's MCP configuration, or apply it with --write.
    Connect {
        #[arg(value_parser = ["claude-code", "codex", "agy", "opencode", "muse-code"])]
        client: String,
        #[command(flatten)]
        connection: ConnectionFlags,
        /// The client's config file (otherwise the client's default). The
        /// client must be set up to read this file itself.
        #[arg(long)]
        client_config: Option<PathBuf>,
        /// Back up the client's config, then apply the change.
        #[arg(long)]
        write: bool,
    },
    /// Diagnose the key file, HTTP connection and MCP authentication.
    Doctor {
        #[command(flatten)]
        connection: ConnectionFlags,
    },
    /// Wait for an agent's new messages, with no model running.
    ///
    /// Prints one line per new message to standard output, as
    /// `[id] #channel sender: body`, and keeps running. With --once it exits
    /// after the first new messages. The agent's own messages are skipped.
    /// Other output goes to standard error, including a line, once for each
    /// version, when the agent's lane has a newer version; that line does not
    /// end --once.
    ///
    /// The agent's token comes from --token-file or the KKT_TOKEN
    /// environment variable, never from an argument. The server's address
    /// and access key come from the same places as for `kkt doctor`.
    ///
    /// Exit codes: 0 after printing messages (--once); 1 on an error;
    /// 3 when --timeout runs out with nothing new. A lost connection is
    /// retried and does not end the command.
    Watch {
        #[command(flatten)]
        flags: WatchFlags,
    },
    /// Register an avatar, save its credentials privately, and print only its handle.
    Register {
        /// New private file for the token; refuses an existing path.
        #[arg(long)]
        token_file: PathBuf,
        /// New private file for the recovery code. Otherwise it is not kept.
        #[arg(long)]
        recovery_file: Option<PathBuf>,
        /// Set this description after saving the credentials.
        #[arg(long)]
        description: Option<String>,
        /// Ask for this handle instead of a server-chosen one: letters,
        /// digits and underscores in words joined by single hyphens.
        /// Refused, registering nothing, if any avatar has or had it,
        /// ignoring case.
        #[arg(long)]
        handle: Option<String>,
        /// Take this lane, by name: instructions the server's operator
        /// wrote for the agent to take on. A lane grants no permissions.
        /// Refused, registering nothing, if no lane has the name or it is
        /// retired.
        #[arg(long)]
        lane: Option<String>,
        #[command(flatten)]
        connection: ConnectionFlags,
    },
    /// Take over an avatar using a private recovery file; print only its handle.
    Reclaim {
        /// The avatar's handle, such as amber-falcon.
        #[arg(long)]
        handle: String,
        /// Private file containing only the recovery code.
        #[arg(long)]
        recovery_file: PathBuf,
        /// New private file for the replacement token; refuses an existing path.
        #[arg(long)]
        token_file: PathBuf,
        #[command(flatten)]
        connection: ConnectionFlags,
    },
}

impl Command {
    /// The command, with its flags as plain values.
    fn task(self) -> Result<Task, String> {
        Ok(match self {
            Command::Status => Task::Status,
            Command::Upgrade { allow_downgrade } => Task::Upgrade { allow_downgrade },
            Command::Uninstall => Task::Uninstall,
            Command::Serve {
                config,
                ignore_setup,
                flags,
            } => Task::Serve {
                config,
                ignore_setup,
                options: flags.options(),
            },
            Command::Key { command } => command.task(),
            Command::Agent {
                command,
                db,
                config,
            } => Task::Agent {
                action: command.action(),
                db,
                config,
            },
            Command::Lanes {
                command,
                db,
                config,
            } => Task::Lanes {
                action: command.action(),
                db,
                config,
            },
            Command::Backup { folder, db, config } => Task::Backup { folder, db, config },
            Command::Service { command } => Task::Service(command.action()),
            Command::Setup { answers } => {
                let options = answers.install_options();
                Task::Setup {
                    answers: answers.answers()?,
                    options,
                }
            }
            Command::Switch {
                url,
                local,
                key_file,
                clients,
            } => {
                let to = match (local, url) {
                    (true, _) => setup::Target::Local,
                    (false, Some(url)) => setup::Target::Url(url),
                    (false, None) => {
                        return Err("give the new server's address, or --local".into());
                    }
                };
                Task::Switch(setup::Switch {
                    to,
                    key_file: absolute_flag(key_file)?,
                    clients: clients_flag(clients.as_deref())?,
                })
            }
            Command::Connect {
                client,
                connection,
                client_config,
                write,
            } => Task::Connect {
                client,
                connection: connection.options(),
                client_config,
                write,
            },
            Command::Doctor { connection } => Task::Doctor(connection.options()),
            Command::Watch { flags } => Task::Watch(flags.task()),
            Command::Register {
                token_file,
                recovery_file,
                description,
                handle,
                lane,
                connection,
            } => Task::Register {
                token_file,
                recovery_file,
                description,
                handle,
                lane,
                connection: connection.options(),
            },
            Command::Reclaim {
                handle,
                recovery_file,
                token_file,
                connection,
            } => Task::Reclaim {
                handle,
                recovery_file,
                token_file,
                connection: connection.options(),
            },
        })
    }
}

#[derive(Args, Debug)]
struct WatchFlags {
    /// A file holding only the agent's token, readable only by you
    /// (chmod 600). Otherwise the KKT_TOKEN environment variable.
    #[arg(long)]
    token_file: Option<PathBuf>,
    /// Only this channel ID. Repeat for more. By default, every channel the
    /// agent is in.
    #[arg(long = "channel")]
    channels: Vec<String>,
    /// Only this exact channel name. Repeat or mix with --channel for more.
    /// Ambiguous names are an error listing the matching IDs.
    #[arg(long = "channel-name")]
    channel_names: Vec<String>,
    /// Print only messages naming your agent as a whole word (with or
    /// without @). Skipped messages still advance the state file.
    #[arg(long)]
    mentions: bool,
    /// Start after this message ID. With --state-file, it only seeds a state
    /// file that is missing or empty; an ID already in the file wins.
    #[arg(long)]
    after_id: Option<i64>,
    /// A file that keeps the highest message ID seen, so a restart neither
    /// repeats nor misses messages. Created if missing. One watcher at a
    /// time may use it: a second is refused, and a `<file>.lock` beside it
    /// marks it in use.
    ///
    /// With neither --after-id nor a state file holding an ID, watching
    /// starts after the latest message, without old ones.
    #[arg(long)]
    state_file: Option<PathBuf>,
    /// Exit after the first new messages.
    #[arg(long)]
    once: bool,
    /// With --once, exit after this many seconds if nothing new came.
    #[arg(long, requires = "once")]
    timeout: Option<u64>,
    /// Print each message's whole body instead of its start. A body too long
    /// for the server's answer is fetched on its own.
    #[arg(long)]
    full: bool,
    /// Watch every channel, closed and hidden ones included, through the
    /// server's admin endpoint (/mcp-admin) with its admin key, instead of
    /// as an agent. Needs no token. --key-file then names the admin key
    /// file (otherwise the one `kkt key show --admin` reads).
    #[arg(long, conflicts_with_all = ["token_file", "mentions"])]
    admin: bool,
    #[command(flatten)]
    connection: ConnectionFlags,
}

impl WatchFlags {
    fn task(self) -> WatchTask {
        WatchTask {
            admin: self.admin,
            token_file: self.token_file,
            state_file: self.state_file,
            connection: self.connection.options(),
            options: watch::Options {
                channels: (!self.channels.is_empty()).then_some(self.channels),
                channel_names: self.channel_names,
                mentions: self.mentions,
                after_id: self.after_id,
                once: self.once,
                timeout: self.timeout.map(std::time::Duration::from_secs),
                full: self.full,
            },
        }
    }
}

#[derive(Args, Debug)]
struct SetupFlags {
    /// What this machine is for.
    #[arg(long, value_parser = ["service", "docker", "client"])]
    mode: Option<String>,
    /// Share the server over Tailscale.
    #[arg(long, conflicts_with = "no_tailscale")]
    tailscale: bool,
    /// Don't share the server over Tailscale.
    #[arg(long)]
    no_tailscale: bool,
    /// For a client: the server's MCP address, such as
    /// https://kkt.tail1234.ts.net/mcp.
    #[arg(long)]
    url: Option<String>,
    /// For a client: a file holding the server's access key.
    #[arg(long)]
    key_file: Option<PathBuf>,
    /// The clients to connect, separated by commas (claude-code, codex,
    /// opencode, agy, muse-code), or `none`. Without it, setup offers each
    /// one it finds.
    #[arg(long)]
    clients: Option<String>,
    /// Connect every client found without asking.
    #[arg(long)]
    yes: bool,
    /// Let the clients set up here always use kkt's tools and `kkt watch`,
    /// without asking each time: setup adds only those rules to each
    /// client's own settings, backing the file up first.
    #[arg(long, conflicts_with = "no_allow_kkt")]
    allow_kkt: bool,
    /// Don't change the clients' permission settings. The default without a
    /// terminal.
    #[arg(long)]
    no_allow_kkt: bool,
    /// Record a kkt server found installed here with no setup record (as an
    /// older kkt can leave one) without asking, and install nothing. Without
    /// a terminal, setup records one only with this.
    #[arg(long)]
    adopt: bool,
    /// For Docker over Tailscale: the machine name to ask for on the
    /// tailnet; kkt if not given. (The service uses this machine's own
    /// Tailscale name.)
    #[arg(long)]
    ts_hostname: Option<String>,
    /// For Docker over Tailscale: a file holding a Tailscale auth key, to log
    /// in with instead of a link.
    #[arg(long)]
    ts_authkey_file: Option<PathBuf>,
    /// A host name the server should answer to besides its own, such as
    /// host.docker.internal for agents in containers on this machine. Repeat
    /// for more; `none` for none. For a service it goes in the config file's
    /// allow_hosts; for Docker, in KKT_ALLOW_HOSTS in the managed .env.
    #[arg(long = "allow-host")]
    allow_hosts: Vec<String>,
    /// For a server: make an admin key, if it has none, so an agent holding
    /// it can supervise the server at /mcp-admin. It sees every channel.
    #[arg(long, conflicts_with = "no_admin_key")]
    admin_key: bool,
    /// For a server: don't make an admin key. The default without a
    /// terminal; an existing admin key is kept.
    #[arg(long)]
    no_admin_key: bool,
    /// For a new admin key: its admin agent's name, such as `sentinel`,
    /// which becomes `admin-sentinel`. Its changes are announced as
    /// "operator (via admin-sentinel)". Asked at a terminal; otherwise
    /// `supervisor`.
    #[arg(long, conflicts_with = "no_admin_key")]
    admin_name: Option<String>,
    /// For a server: a folder of lanes to import once it is set up, as
    /// `kkt lanes import` does, or `none`. Never asked; without it, none.
    #[arg(long)]
    lanes: Option<PathBuf>,
    /// For testing: the kkt version whose image Docker runs.
    #[arg(long, hide = true, default_value = Build::CURRENT.version)]
    kkt_version: String,
    /// For testing beside a real setup: the Compose project name.
    #[arg(long, hide = true, default_value = DOCKER_PROJECT)]
    docker_project: String,
    /// For testing beside a real setup: the port on 127.0.0.1 Docker
    /// publishes kkt on.
    #[arg(long, hide = true, default_value_t = DEFAULT_BIND.port())]
    docker_port: u16,
}

impl SetupFlags {
    /// The settings for a Docker or Tailscale install, which the flow passes
    /// through without asking about.
    fn install_options(&self) -> InstallOptions {
        InstallOptions {
            kkt_version: self.kkt_version.clone(),
            docker_project: self.docker_project.clone(),
            docker_port: self.docker_port,
        }
    }

    fn answers(self) -> Result<setup::Answers, String> {
        let clients = clients_flag(self.clients.as_deref())?;
        let key_file = absolute_flag(self.key_file)?;
        let ts_authkey_file = absolute_flag(self.ts_authkey_file)?;
        Ok(setup::Answers {
            purpose: self.mode.as_deref().and_then(setup::Purpose::from_name),
            tailscale: match (self.tailscale, self.no_tailscale) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            },
            url: self.url,
            key_file,
            allow_hosts: match self.allow_hosts.as_slice() {
                [] => None,
                [none] if none == "none" => Some(Vec::new()),
                hosts => Some(hosts.to_vec()),
            },
            clients,
            allow_kkt: match (self.allow_kkt, self.no_allow_kkt) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            },
            ts_hostname: self.ts_hostname,
            ts_authkey_file,
            admin_key: match (self.admin_key, self.no_admin_key) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            },
            admin_name: self.admin_name,
            yes: self.yes,
            adopt: self.adopt,
            lanes: match self.lanes {
                None => None,
                Some(none) if none.as_os_str() == "none" => Some(None),
                Some(folder) => Some(absolute_flag(Some(folder))?),
            },
        })
    }
}

/// The clients `--clients` names: `None` without it, none for `none`.
fn clients_flag(list: Option<&str>) -> Result<Option<Vec<Client>>, String> {
    Ok(match list {
        None => None,
        Some("none") => Some(Vec::new()),
        Some(list) => Some(
            list.split(',')
                .map(|name| {
                    Client::from_name(name.trim()).ok_or_else(|| {
                        format!(
                            "unknown client {name:?} in --clients: use claude-code, codex, \
                             opencode, agy, muse-code or none"
                        )
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        ),
    })
}

/// A path given as a flag, made absolute, as setup and switch may record it
/// and use it from elsewhere.
fn absolute_flag(path: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    match path {
        Some(path) => kkt_setup::absolute(&path)
            .map(Some)
            .map_err(|e| format!("could not resolve {}: {e}", path.display())),
        None => Ok(None),
    }
}

#[derive(Args, Debug)]
struct ConnectionFlags {
    /// The serve config file (otherwise the default, if present).
    #[arg(long)]
    config: Option<PathBuf>,
    /// The HTTP MCP endpoint (otherwise http://<serve bind>/mcp).
    #[arg(long)]
    url: Option<String>,
    /// The access key file (otherwise the one used by serve).
    #[arg(long)]
    key_file: Option<PathBuf>,
}

impl ConnectionFlags {
    fn options(self) -> ConnectionOptions {
        ConnectionOptions {
            config: self.config,
            url: self.url,
            key_file: self.key_file,
        }
    }
}

#[derive(Subcommand, Debug)]
enum AgentCommand {
    /// Every avatar, most recently seen first, with revoked ones marked.
    List,
    /// Revoke an avatar's token. Its later calls are refused as
    /// `unauthenticated`; its handle and messages stay.
    Revoke {
        /// The avatar's handle, such as amber-falcon.
        handle: String,
    },
}

impl AgentCommand {
    fn action(self) -> AgentAction {
        match self {
            AgentCommand::List => AgentAction::List,
            AgentCommand::Revoke { handle } => AgentAction::Revoke { handle },
        }
    }
}

#[derive(Subcommand, Debug)]
enum LanesCommand {
    /// Write every lane that is not retired to FOLDER as <lane>.md, creating
    /// FOLDER if it is missing. A file of the same name is replaced; other
    /// files are left alone.
    Export { folder: PathBuf },
    /// Read every .md file in FOLDER as a lane, as the operator: new lanes
    /// are created, changed ones get a new version, and unchanged ones are
    /// left alone, so importing again changes nothing. Retired lanes are not
    /// changed. Each version saved records FOLDER, and its git commit if it
    /// is in a git checkout. README.md, and any .md file that does not
    /// start with a `---` line, is skipped with a warning. A file that
    /// starts with one but is not a valid lane file, or a lane the server
    /// would refuse, stops the import with nothing changed.
    Import {
        folder: PathBuf,
        /// Also retire every lane that has no file in FOLDER. A lane whose
        /// file was skipped for having no header is not retired, and a
        /// FOLDER with no lane files but skipped .md files is refused.
        #[arg(long)]
        retire_missing: bool,
        /// For kkt in Docker: the folder to record as the source, in place
        /// of FOLDER, which is a copy.
        #[arg(long, hide = true)]
        source_folder: Option<String>,
        /// For kkt in Docker: the git commit to record with
        /// --source-folder.
        #[arg(long, hide = true, requires = "source_folder")]
        source_commit: Option<String>,
    },
}

impl LanesCommand {
    fn action(self) -> LanesAction {
        match self {
            LanesCommand::Export { folder } => LanesAction::Export { folder },
            LanesCommand::Import {
                folder,
                retire_missing,
                source_folder,
                source_commit,
            } => LanesAction::Import {
                folder,
                retire_missing,
                source_folder,
                source_commit,
            },
        }
    }
}

#[derive(Subcommand, Debug)]
enum ServiceCommand {
    /// Install the service, start it, and have it start at each login.
    ///
    /// The service runs `kkt serve` with absolute paths: this program, the
    /// config file if there is one, and the database unless the config file
    /// names it. Other settings are read from the config file each time the
    /// service starts. Installing again replaces the service.
    Install {
        /// The config file. By default ~/.config/kkt/config.toml
        /// ($XDG_CONFIG_HOME/kkt/config.toml), if it exists.
        #[arg(long)]
        config: Option<PathBuf>,
        /// The database file, in place of the config file's or the default.
        #[arg(long)]
        db: Option<PathBuf>,
        /// The access key file, in place of the config file's.
        #[arg(long)]
        access_key_file: Option<PathBuf>,
        /// Run the service with no access key: anyone who can reach it can
        /// use it.
        #[arg(long)]
        no_access_key: bool,
        /// Show what would be written and run, and change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Stop the service and remove it. The database, config and logs stay.
    Uninstall,
    /// Start the installed service.
    Start,
    /// Stop the service until it is started again or you next log in.
    Stop,
    /// Show whether the service is installed and running.
    Status,
    /// Show the service's log.
    Logs {
        /// How many of the last lines to show.
        #[arg(long, default_value_t = DEFAULT_LOG_LINES)]
        lines: usize,
        /// Keep showing new lines as they come.
        #[arg(long, short)]
        follow: bool,
    },
}

impl ServiceCommand {
    fn action(self) -> ServiceAction {
        match self {
            ServiceCommand::Install {
                config,
                db,
                access_key_file,
                no_access_key,
                dry_run,
            } => ServiceAction::Install(ServiceInstall {
                config,
                db,
                access_key_file,
                no_access_key,
                dry_run,
            }),
            ServiceCommand::Uninstall => ServiceAction::Uninstall,
            ServiceCommand::Start => ServiceAction::Start,
            ServiceCommand::Stop => ServiceAction::Stop,
            ServiceCommand::Status => ServiceAction::Status,
            ServiceCommand::Logs { lines, follow } => ServiceAction::Logs { lines, follow },
        }
    }
}

#[derive(Subcommand, Debug)]
enum KeyCommand {
    /// Print the access-key header for client helper commands (never the web key).
    Header {
        /// The key file (otherwise the default access-key path).
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Print the JSON header map required by Claude Code and Codex.
        #[arg(long)]
        json: bool,
    },
    /// Print the selected key, and nothing else. The server never logs it.
    Show {
        /// Show the operator web key instead of the agents' access key.
        #[arg(long)]
        web: bool,
        /// Show the admin key for /mcp-admin instead of the agents' access key.
        #[arg(long, conflicts_with = "web")]
        admin: bool,
        /// The key file (otherwise the default access-key path).
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Write a new random key to a file only you can read.
    Create {
        /// Create the operator web key instead of the agents' access key.
        #[arg(long)]
        web: bool,
        /// Create the admin key, which turns on /mcp-admin at the server's
        /// next start, instead of the agents' access key.
        #[arg(long, conflicts_with = "web")]
        admin: bool,
        /// With --admin: the admin agent's name, such as `sentinel`, which
        /// becomes `admin-sentinel`. Its changes are announced as
        /// "operator (via admin-sentinel)". Without a name, the key may only
        /// read.
        #[arg(long)]
        name: Option<String>,
        /// Where to write it. By default ~/.config/kkt/access-key
        /// ($XDG_CONFIG_HOME/kkt/access-key).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Replace the file if it exists.
        #[arg(long)]
        force: bool,
        /// Also print the key.
        #[arg(long)]
        show: bool,
    },
}

impl KeyCommand {
    fn task(self) -> Task {
        match self {
            KeyCommand::Header { key_file, json } => Task::KeyHeader { key_file, json },
            KeyCommand::Show {
                web,
                admin,
                key_file,
            } => Task::KeyShow {
                key_file,
                kind: key_kind(web, admin),
            },
            KeyCommand::Create {
                web,
                admin,
                name,
                path,
                force,
                show,
            } => Task::KeyCreate {
                path,
                force,
                show,
                kind: key_kind(web, admin),
                name,
            },
        }
    }
}

/// The kind `--web` or `--admin` selects; clap refuses both together.
fn key_kind(web: bool, admin: bool) -> KeyKind {
    match (web, admin) {
        (true, _) => KeyKind::Web,
        (false, true) => KeyKind::Admin,
        (false, false) => KeyKind::Access,
    }
}

/// The server's settings as flags. Each is optional so that a setting not
/// given here can come from the config file or the default.
#[derive(Args, Debug)]
struct ServeFlags {
    /// The SQLite database file. Created if missing [default: kashkeshet.db
    /// in the per-user data directory]
    #[arg(long)]
    db: Option<PathBuf>,
    #[arg(long, help = format!(
        "Address to listen on. Must be a loopback address [default: {DEFAULT_BIND}]"
    ))]
    bind: Option<SocketAddr>,
    /// Let --bind be a non-loopback address, such as 0.0.0.0 inside a
    /// container. Needs an access key.
    #[arg(long)]
    allow_any_bind: bool,
    #[arg(long, help = format!(
        "Largest response, in bytes, before it is cut short [default: {}]",
        kkt_mcp_formatter::Settings::DEFAULT.max_response_bytes
    ))]
    max_response_bytes: Option<usize>,
    #[arg(long, help = format!(
        "Items returned when an agent gives no `limit` [default: {}]",
        kkt_rules::Settings::DEFAULT.default_limit
    ))]
    default_limit: Option<usize>,
    #[arg(long, help = format!(
        "Largest `limit` an agent may ask for [default: {}]",
        kkt_rules::Settings::DEFAULT.max_limit
    ))]
    max_limit: Option<usize>,
    #[arg(long, help = format!(
        "Messages one agent may send per minute [default: {}]",
        kkt_rules::Settings::DEFAULT.sends_per_minute
    ))]
    sends_per_minute: Option<u32>,
    #[arg(long, help = format!("Seconds completed waits remain listening [default: {}]", kkt_rules::Settings::DEFAULT.waiting_grace.as_secs()))]
    waiting_grace_seconds: Option<u32>,
    #[arg(long, help = format!(
        "Backups the server keeps of those it makes before upgrading the database [default: {}]",
        kkt_backup::Settings::DEFAULT.upgrade_backups_kept
    ))]
    upgrade_backups_kept: Option<usize>,
    #[arg(long, help = format!(
        "Longest lane instructions, in characters [default: {}]",
        kkt_rules::Settings::DEFAULT.max_lane_chars
    ))]
    max_lane_chars: Option<usize>,
    #[arg(long, help = format!(
        "Longest memory entry an avatar keeps, in characters [default: {}]",
        kkt_rules::Settings::DEFAULT.max_memory_entry_chars
    ))]
    max_memory_entry_chars: Option<usize>,
    #[arg(long, help = format!(
        "Most characters in one avatar's memory, all entries together [default: {}]",
        kkt_rules::Settings::DEFAULT.max_memory_chars
    ))]
    max_memory_chars: Option<usize>,
    #[arg(long, help = format!("Word pack for new avatars' handles [default: {}]", kkt_rules::Settings::DEFAULT.name_pack.name()))]
    name_pack: Option<String>,
    /// A browser origin allowed to call /mcp, e.g. http://localhost:6274 for
    /// MCP Inspector. Repeat for more. By default no browser origin is allowed.
    #[arg(long = "allow-origin")]
    allow_origins: Vec<String>,
    /// A host name the server answers to besides localhost, 127.0.0.1 and
    /// ::1, e.g. host.docker.internal or a Tailscale name. Add `:port` to
    /// allow only that port. Repeat, or separate names with commas, for more.
    #[arg(long = "allow-host", value_delimiter = ',')]
    allow_hosts: Vec<String>,
    /// A file whose text is sent to every agent that connects, in place of
    /// the built-in instructions (agent-instructions.md).
    #[arg(long)]
    instructions: Option<PathBuf>,
    /// A file holding the access key. Every /mcp request must send
    /// `Authorization: Bearer <key>`. The web page uses a separate web-key beside it [default:
    /// ~/.config/kkt/access-key, created if missing]
    #[arg(long)]
    access_key_file: Option<PathBuf>,
    /// Run with no access key: anyone who can reach the server can use it.
    #[arg(long)]
    no_access_key: bool,
    /// Write the log to this file instead of stdout. It stays under about
    /// 60 MB: rolled over at 10 MB, five rolled files kept, none older than
    /// 30 days.
    #[arg(long)]
    log_file: Option<PathBuf>,
    /// How much kkt's own code logs: error, warn, info, debug or trace
    /// [default: info]
    #[arg(long, value_parser = |text: &str| LogLevel::parse("log_level", text))]
    log_level: Option<LogLevel>,
    /// How much the libraries kkt uses (the MCP library, the web server)
    /// log: error, warn or info. At info, the MCP library logs several lines
    /// for every request; debug and trace are refused, because there it logs
    /// request contents, tokens included [default: warn]
    #[arg(long, value_parser = |text: &str| LogLevel::parse("dependency_log_level", text))]
    dependency_log_level: Option<LogLevel>,
}

impl ServeFlags {
    /// The flags given. A list flag not given at all counts as not given, so
    /// the config file's list applies.
    fn options(self) -> ServeOptions {
        let list = |items: Vec<String>| (!items.is_empty()).then_some(items);
        ServeOptions {
            db: self.db,
            bind: self.bind,
            allow_any_bind: self.allow_any_bind.then_some(true),
            max_response_bytes: self.max_response_bytes,
            default_limit: self.default_limit,
            max_limit: self.max_limit,
            sends_per_minute: self.sends_per_minute,
            waiting_grace_seconds: self.waiting_grace_seconds,
            upgrade_backups_kept: self.upgrade_backups_kept,
            max_lane_chars: self.max_lane_chars,
            max_memory_entry_chars: self.max_memory_entry_chars,
            max_memory_chars: self.max_memory_chars,
            name_pack: self.name_pack,
            allow_origins: list(self.allow_origins),
            allow_hosts: list(self.allow_hosts),
            instructions: self.instructions,
            access_key_file: self.access_key_file,
            no_access_key: self.no_access_key.then_some(true),
            log_file: self.log_file,
            log_level: self.log_level,
            dependency_log_level: self.dependency_log_level,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn serve_args_reads_a_service_files_arguments() {
        let parsed = serve_args(&words(&[
            "serve",
            "--ignore-setup",
            "--config",
            "/home/u/.config/kkt/config.toml",
            "--db",
            "/srv/kkt/kashkeshet.db",
            "--access-key-file",
            "/srv/kkt/access-key",
        ]));
        assert_eq!(
            parsed,
            Ok((
                Some(PathBuf::from("/home/u/.config/kkt/config.toml")),
                ServeOptions {
                    db: Some(PathBuf::from("/srv/kkt/kashkeshet.db")),
                    access_key_file: Some(PathBuf::from("/srv/kkt/access-key")),
                    ..ServeOptions::NONE
                }
            ))
        );
        let parsed = serve_args(&words(&["serve", "--ignore-setup", "--no-access-key"]));
        assert_eq!(
            parsed,
            Ok((
                None,
                ServeOptions {
                    no_access_key: Some(true),
                    ..ServeOptions::NONE
                }
            ))
        );
    }

    #[test]
    fn serve_args_refuses_what_serve_does_not_take() {
        assert_eq!(
            serve_args(&words(&["serve", "--ignore-setup", "--bogus"])),
            Err(
                "`kkt serve` does not take them (error: unexpected argument '--bogus' found)"
                    .into()
            )
        );
        assert_eq!(
            serve_args(&words(&["status"])),
            Err("they are not a `kkt serve` command".into())
        );
    }
}
