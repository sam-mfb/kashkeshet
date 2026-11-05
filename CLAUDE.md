# Working on kashkeshet

The README explains what kkt is and how to use it. This file is for working
on the code. Read [PRINCIPLES.md](PRINCIPLES.md) first: it states the coding
principles and the rules for organizing code into parts and crates. This
file adds what is specific to kkt.

## Design principles

1. **Simple.** Understanding kkt, starting a server and connecting agents
   must be as simple and as hard to get wrong as possible.
2. **Not opinionated.** kkt provides mechanics: avatars, channels,
   messages, and waiting for them. It does not tell developers or agents how
   to work; they find their own patterns in chat. The test for a feature:
   does it add a general mechanism, or prescribe a way of working? Only the
   first belongs in kkt.
3. **Existing installs keep working.** kkt is in use, so a new version must
   work with what an older one left behind. This covers at least: the setup
   record, the managed Docker folder and its Compose files, service files,
   keys and key files, the database, client configs written by
   `kkt connect`, CLI commands and flags, and the MCP tools agents see. A change
   may break an existing install only when there is no other choice, and
   only with Sam's explicit approval, stated in the pull request.

## Where code lives

kkt is a workspace: one crate per concern under `crates/`, and the program,
the root package `kashkeshet` (the `kkt` binary and its library). Each
crate, and each directory or file of the program, says at its top what it
owns. Put new code in the crate that owns its area. If none fits, add a
crate named for the area (e.g. `kkt-search`), not for a kind of code
(`types`, `utils`) or the library behind it.

Features follow PRINCIPLES.md rule 8: `real` holds what touches the machine,
`testing` holds the shared stand-in. A crate with no feature listed has
none.

| Crate | Owns | Must not | Features |
|---|---|---|---|
| `kkt-rules` | Every permission decision and every change, for agents and the human | Reach the database except through `Reads` and `Writes`; format output | none |
| `kkt-store` | The `Reads`/`Writes` contract and its SQLite implementation; the schema (`migrations/`) | Decide permissions; use rusqlite or SQL outside `sqlite.rs` | `real`: `Db::open`, `pending_upgrade`, `copy_database`, which open database files. `testing`: `Db::open_in_memory`, `Db::execute_for_tests` |
| `kkt-format` | Turning rule results into text or JSON | Decide permissions; read the database | none |
| `kkt-mcp` | The agent and admin MCP tools; what the two share; the endpoints' HTTP transport; their text | Touch the database except through the rules; hold tool or parameter text in Rust | none |
| `kkt-web` | The human's pages, through its own contract (`src/source.rs`, `Source` and `Gate`) | Depend on the rules, the store or admission: `kkt-server` adapts them to its contract | `testing`: the page scenarios every source must pass (`kkt_web::testing`); turns on `tower` and tokio's `time` |
| `kkt-admission` | Whether an HTTP caller is let in at all; web sessions; the checks on the server's keys | Know about agent tokens or channel access | none |
| `kkt-hosts` | Which host names the server answers to | | none |
| `kkt-server` | Assembling the server from its parts and running it; recording each server run; the server's adapters to the web contract | Make decisions: it only wires parts together | `real`: `listen` and `Listening`, which bind the server's socket and serve on it (turns on `tokio`) |
| `kkt-config` | Every limit, fixed setting and default, and their checks | Read files or the environment | none |
| `kkt-settings` | Gathering a server's settings from flags, config file and defaults | Read files or the environment: callers pass their text and values in | none |
| `kkt-provenance` | What produced a server run, and the build script that records the git commit | Store the run record: `kkt-server` does | none |
| `kkt-clock` | Time: `Clock`, and kkt's time types `Time` and `Span`. The only crate that uses chrono | Let a chrono type into its public surface | `real`: `SystemClock`, the real clock and its sleep; turns on tokio's `time`. `testing`: `FixedClock` |
| `kkt-names` | Handles, IDs, randomness (`Entropy`) and the name packs | Draw randomness except through `Entropy` | `real`: `OsEntropy` (turns on `rand`). `testing`: `SeqEntropy`, `SameHandleEntropy` |
| `kkt-events` | Notices of new messages, agents, channels and members | Let a tokio type out of the crate | none |
| `kkt-shutdown` | The stop signal | Let a tokio-util type out of the crate | `real`: `StopSignals`, the operating system's stop signals |
| `kkt-logs` | The server's log file, rolled over within its policy | | `real`: all of it |
| `kkt-setup` | `kkt setup`: what this machine is set up as, the setup record, and what follows from it for status, upgrade, lanes, backup and key files | Read files or run anything: the program does, and carries out what is decided here | none |
| `kkt-container` | Running kkt in Docker: its Compose files, its folder, and the commands that manage it | Decide what a person wants or remember what was set up; touch the machine except through `Host` | `real`: `OsHost` |
| `kkt-service` | Running kkt as a service (launchd, systemd), and the plan of what the service runs | Touch the machine except through `System` | `real`: `OsSystem` |
| `kkt-remote-access` | Sharing a service-mode server over Tailscale | Change the Tailscale login, DNS preferences, kkt's config or its service | `real`: `OsHost`, which runs `tailscale` |
| `kkt-client-config` | Client MCP configs and their file formats | | `real`: writing configs and finding where they are, whether a client is installed and connected |
| `kkt-diagnostics` | `kkt doctor`'s checks and their plan; setup's wait for a new server | Change anything; touch the machine except through `Prober` | `real`: `OsProber`, the key file reads and probes (turns on `reqwest`) |
| `kkt-backup` | Backup folders: names, contents, file modes, pruning | Read the clock or copy the database itself: the caller passes both | `real`: `write`, `prune` |
| `kkt-lane-files` | Lanes as files: the `<lane>.md` format and the folder's git commit | Touch the disk except through `Disk` | `real`: `OsDisk` |
| `kkt-watch` | `kkt watch` | Reach the server, the time, its place or its output except through what is injected | none |
| `kkt-agent-client` | The shared MCP client for agent commands and watching | | `real`: `Client`, `Remote`, `AdminRemote`, which reach the server (turns on `reqwest` and `rmcp`) |
| `kkt-scratch` | Tests' scratch folders under `target/tmp` | Be anything but a dev-dependency | none |

The program, in the root package:

| Part | Owns | Must not | Features |
|---|---|---|---|
| `src/bin/kkt/` (`main.rs`, `cli.rs`) | The `kkt` command: subcommands and the files and environment they read; `cli.rs` wraps clap and holds every clap type | Pass a clap type out of `cli.rs` | Turns on `real` in every crate that has it |
| `src/machine/` (in the library `src/lib.rs`) | What kkt keeps on this machine: serve settings and the config file, the setup record, key files, how a client reaches the server, agent credential files; `credentials.rs` wraps libc | | |
| `src/commands/` (in the library) | One module per command, each with a `run` taking plain inputs: `serve`, `service`, `setup` (with `OsSteps` and `OsSwitchSteps`, setup's real steps, in `setup/steps.rs`), `doctor`, `lifecycle` (status, upgrade, uninstall and backup, with the backup `serve` also makes), `key`, `agent` (register, reclaim and `kkt agent`), `lanes` (with the lane helpers `setup` also uses), `watch` (with `WatchOutput`) and `connect` | Take a clap type | |
| `src/terminal/`, `src/logging.rs`, `src/process.rs` (in the library) | Asking setup's questions and the Tailscale login link on the terminal; starting the log; this process's environment, closed-output setting, program path, and writing a file whole; `logging.rs` wraps `tracing_subscriber`, `process.rs` wraps libc | | |

Other files: `crates/kkt-store/migrations/` (the schema, one numbered file per change);
`crates/kkt-mcp/agent-instructions.md` and `crates/kkt-mcp/admin-instructions.md` (what agents are told
when they connect); `crates/kkt-mcp/agent-tools.toml` and `crates/kkt-mcp/admin-tools.toml` (what agents
are told about each tool); `crates/kkt-mcp/build.rs` (builds the tool text); `tests/contract/` (the tools agents see, pinned);
`CHANGELOG.md` (see `docs/releases.md`).

## Rules

**Storage.** Only `crates/kkt-store/src/sqlite.rs` imports rusqlite or contains SQL.
Rules ask for what they need through named methods on `Reads` or `Writes`.
A rule that must be atomic runs as one `Db::write`. Schema changes go in a
new numbered file in `crates/kkt-store/migrations/`. Migrations only add; never edit one that
has shipped.

**Permissions.** Every check on whether an agent or the human may use a
channel goes through `kkt_rules::access::decide`. To an agent outside a closed
channel, the channel does not exist: the answer is `not_found`, exactly as
for a missing ID.

**The human.** The human operator is an actor in `rules`, beside agents. A
change the human makes goes through the same rule as an agent's, and a
change to a channel posts a message in it from the operator saying what
changed, in the same `Db::write`.

**What agents see.** The MCP tools are a contract with every agent. Do not
rename or remove tools, parameters, error codes or output fields; add new
ones. Tool and parameter text lives in `crates/kkt-mcp/agent-tools.toml` and
`crates/kkt-mcp/admin-tools.toml`, never in Rust, with limits written as placeholders such
as `{max_limit}`. After an intended change, run
`UPDATE_CONTRACT=1 cargo test --test contract` and review the diff to
`tests/contract/`. Keep `crates/kkt-mcp/agent-instructions.md` in step with the tools.

**kkt's names for the principles.**
- Time comes only from the injected `Clock` (clippy and the crate boundaries enforce this), randomness only from `Entropy`.
- Dependencies are injected through `kkt_rules::Deps` or `kkt_server::Parts`. There is
  no trait in front of `Rules`; it has one implementation.
- Every limit and default lives in `crates/kkt-config/src/lib.rs`, is checked at startup, and
  appears in `provenance::settings_text`. Bad settings stop the server with
  an error naming the setting.
- Rules return `RuleError` with a fixed code from `crates/kkt-rules/src/error.rs`. Logging
  happens only at the edges (`src/bin/kkt/main.rs`, `src/machine/`, `crates/kkt-mcp/src/lib.rs`, `crates/kkt-mcp/src/mcp_admin.rs`, `crates/kkt-web/`). `unwrap` and
  `expect` are for tests only.
- Stored rows that record activity carry the `run_id` of the server run that
  made them.
- Third-party types stay inside the part that wraps them.
- axum is the HTTP framework of `kkt-web`, `kkt-mcp` and `kkt-server` only; its types do not leave those three.

**Public surface.** Only the modules the binary and the integration tests
use are public (`lib.rs`). Make an item public only when something outside
needs it.

## Tests

- Run `cargo test`, `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt --check` before every commit. CI runs the same on Ubuntu and
  macOS, and a pull request merges only when they pass.
- A bug fix, a robustness change or a performance claim comes with a test
  that fails without it.
- Every pull request says whether it can affect an existing install, and how
  that was checked. A schema change needs a test that an older database
  migrates (`tests/migrations.rs`).
- Unit tests sit beside their code; rule tests in `crates/kkt-rules/src/tests.rs` use
  `FixedClock` and `SeqEntropy`, never real time. End-to-end tests in
  `tests/` start servers through `tests/common`.
- Tests that run the `kkt` binary give it its own `HOME`. Scratch files go
  under `target/tmp` through `scratch::Dir`, never the system temporary
  folder; CI makes that folder unwritable.

## Working with other agents

Stay inside your own working directory. Share work only through git: push a
branch and open a pull request with `gh`. Do not read or copy another agent's
files. Pull requests target the integration branch the coordinator names. Only
the repository owner merges into `main`.
