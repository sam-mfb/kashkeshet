# Working on kashkeshet

The README explains what kkt is and how to use it. This file is for working
on the code.

## Principles

The general principles are in [principles/](principles/), taken unchanged
from Sam's project template (https://github.com/sam-mfb/project-template).
You do not need to read them all in every session. Read
[ARCHITECTURE.md](principles/ARCHITECTURE.md) and [RUST.md](principles/RUST.md)
when planning, designing or organizing features, and when orchestrating or
supervising work. Read [CODING.md](principles/CODING.md) and RUST.md when
writing code.

This file adds what is specific to kkt. It must not contradict
principles/: where kkt departs from them, this file says so, with the reason
(ARCHITECTURE.md 9.5).

The files in principles/ MUST NOT be edited or committed without human
involvement and approval.

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
4. **Two kinds of line.** Channels, lanes, announcements, instructions and
   memory are soft lines: agents draw and follow them by choice, and nothing
   security-relevant rests on them. Swarms, tokens and ownership are hard
   lines: humans draw them and the server enforces them. Products built on
   these crates may add more, such as orgs and roles. No tool lets an agent
   cross a hard line with the credentials it holds, or gain a credential it
   was not given. Leakage across a hard line is a bug.

## Where code lives

kkt is a workspace: one crate per concern under `crates/`, and the program,
the root package `kashkeshet` (the `kkt` binary and its library). Each
crate, and each directory or file of the program, says at its top what it
owns. Put new code in the crate that owns its area. If none fits, add a
crate named for the area (e.g. `kkt-search`), not for a kind of code
(`types`, `utils`) or the library behind it.

Features: code that touches the machine (files, network, processes, the
clock, randomness, signals) is behind a `real` feature; plain data types
stay outside it. Only programs (binaries and examples) turn on `real`, and
a contract crate never has one. A merged crate (concerns.json's kind
"contract and effect") holds a contract in the open and its one
implementation behind `real`: a crate that depends on it without `real`
depends on the contract alone. Tests SHOULD NOT turn on `real`; a test
that must is a documented exception, listed with its reason in
`.github/scripts/real-exceptions.json` (today, the fake product's tests,
which drive it over HTTP with kkt-agent-client). Stand-ins and other test
support are behind a `testing` feature, which other crates turn on only in
their dev-dependencies (RUST.md 6); a crate's own `testing` may turn on
its dependencies' `testing`, and a dev-only program (see "Dev-only
packages") may turn it on in its dependencies. A fake used only inside one crate stays
in that crate's unit tests. CI builds each crate on its own and checks these
rules (`.github/scripts/check-crates.sh`). A crate with no feature listed
has none.

| Crate | Owns | Must not | Features |
|---|---|---|---|
| `kkt-rules` | Every permission decision and every change, for agents and the human; starting the rules for a server run (`start`, with `Startup` and `NewServerRun`), which records the run first; the core's data, in its storage module (`src/store/`): its questions (`Reader`) and changes (`Writer`) in kkt-db's terms, and the core's tables as kkt-db migrations (`src/store/schema.rs`) | Reach the database except through the storage module's `Reader` and `Writer`; format output; contain SQL or know a particular database (it is given a `kkt_db::Backend`) | `testing`: `testing::swarms_stay_apart_in` (the check every store must pass, on any backend); `testing::scenarios::run_all` (the rules' scenarios whose answers rest on how a database answers and that no end-to-end test asserts on the real backend, run over the backend given: the program's `tests/rules_on_sqlite.rs` runs them on kkt-db-sqlite); `testing::memory` (the rules over an in-memory database, after a test run, for other crates' tests, which set up their data through the kkt-api crates), `testing::database` (the database alone, for a program's own `start`) and `fail_next_write`; `testing::rules_on` (the rules over a database the test opened itself); and `testing::every_store_read`, every store read asked on a database with the `Probes` given, as text, for the program's test that an older release's database answers as it did. No store record, `Reader` or `Writer` can be named outside kkt-rules |
| `kkt-api-agent` | What any registered agent may do: `Api` and its areas (`Agents`, `Channels`, `Messages`, `Memory`, `Lanes`), which `kkt-rules` implements, with `Limits`; and the records every level shares: agents, channels, messages, memory, lanes, the queries, who acts (`Actor`, `Agent`, `Operator`, `Audit`, `Permissions`) and the errors (`RuleError`, `Code`) | | none |
| `kkt-api-operator` | What the human who runs the server may do, by area (`Views`, `Changes`, `Lanes`, `Server`), with the records only they use, and `OperatorApi`, all of it in one swarm; the notices the operator hears as things happen (`Notice`, and `Notices`, which `Views::subscribe` gives); `kkt-rules` implements it | Name kkt-events or any other logic crate | none |
| `kkt-api-admin` | What /mcp-admin's administrator does: the admin reads (`Admin`), creating a swarm (`Swarms`), `AdminChannel`, and `AdminApi`, every operation /mcp-admin uses in one swarm; `kkt-rules` implements it | | none |
| `kkt-db` | The database contract, for any database: values; declaring tables, keys, references, indexes and constraints; the questions and changes a store may make (a narrow language, not SQL); transactions (a read sees one state, a write runs as if alone); search: what a search asks for (`Search`, `Excerpt`) and finds (`Hit`); migrations as additions, recorded per named set; `Legacy`, a database from before migrations were tracked, in no backend's terms; the `Backend` and `Evolve` traits a backend implements, and `BackendExt`, the typed read and write a store calls on any backend | Know a product's tables; name a database or use a database library, except in its `testing` stand-in `FastPlace` | `testing`: `MemoryPlace`, a database in memory that keeps the contract, plainly and slowly; `FastPlace`, a fast stand-in for stores' tests, built on SQLite in memory with a translation of its own (it shares nothing with kkt-db-sqlite); `conformance`, the checks every backend must pass, over a `Place` that gives it fresh databases; `FaultyBackend`, a backend that wraps another and fails a change on cue |
| `kkt-db-sqlite` | kkt-db over SQLite, the only crate that knows SQLite outside tests: SQL written from kkt-db's operations, one writer behind a lock and read-only readers, constraints as triggers, search indexes as FTS5 over kkt-search's words, migrations recorded in `kkt_db_versions`, adopting a database from before kkt-db as the caller's kkt-db `Legacy` describes, reading SQLite's `user_version` itself (`SqlitePlace::adopting`), `copy_database` | Know a product's tables; let a rusqlite type out of the crate | `real`: `SqlitePlace` (including `create`, which refuses an existing path), `CreateError` and `copy_database`, which open database files. `testing`: `MemoryPlace`, a SQLite database in memory |
| `kkt-search` | Search as text: the words of a text (FTS5 `unicode61`'s rule, as a table of our own), a search's terms, whether a text matches them, and an excerpt around the matches, drawn with the marks and window it is given | Know a database | none |
| `kkt-mcp-formatter` | The MCP tools' results turned into text or JSON, within the response size cap; its own `Settings`, bounds (`Problem`, `Settings::problems`) and run-record lines (`Settings::record_lines`) | Decide permissions; read the database | none |
| `kkt-mcp` | The agent and admin MCP tools; what the two share; the endpoints' HTTP transport; their text | Touch the database except through the rules; hold tool or parameter text in Rust; decide who a caller is except through `kkt_identity::Identify` | none |
| `kkt-identity` | Who a caller is, as a contract: `Identify`, which turns an admitted caller's `Principal` into (swarm, actor) or a refusal; the one place that decides who someone is and what permissions they get | Touch HTTP: each surface reads the principal off its own requests | `testing`: `FixedIdentify`, a stand-in `Identify` |
| `kkt-identity-keymode` | Key mode's identity: `KeyMode`, which implements `Identify` from the key that let a caller in and the avatar's token, in the one swarm | Touch HTTP | none |
| `kkt-web` | The human's pages, over kkt-api-operator's `OperatorApi` in the viewer's swarm, rendering the kkt-api crates' records; its own contract is only `Gate` (`src/gate.rs`), which says who is viewing each request (a `Viewer`: a swarm and an operator) and whom a key signs in; the limits only the page applies (`PageLimits`); web sessions (`Sessions`: IDs, hashing, expiry, sign-out and form tokens, holding what the gate said at sign-in), with how long they last in its `Settings`, and its settings' bounds (`Problem`, `Settings::problems`) and run-record lines (`Settings::record_lines`) | Depend on the rules, the store, admission or identity: `kkt-server` adapts them to its gate | `testing`: the page scenarios every implementation of `OperatorApi` must pass (`kkt_web::testing`, over its `App`, with `Door`, a gate and its sessions); turns on `tower`, tokio's `time` and kkt-log's `testing` |
| `kkt-admission` | Whether an HTTP caller is let in at all, and as which `Principal`: no key, which of the server's keys, or (for another program's `Identify` to judge; kkt never sets it) any bearer token; for the web page, whether sign-in is needed and whom a key admits (`WebAdmission`; the sessions are kkt-web's); the checks on the server's keys and what to say when one is refused (`key_refusal`); which access key a server uses (`AccessKey`); where the access key file goes by default (`access_key_file`, in the config folder it is given), and where the web key, the admin key and the admin name's file go beside it (`web_key_path`, `admin_key_path`, `admin_name_path`); binding names to admin keys and the operator advice (`server_keys::bound_to`, `server_keys::bound_in`, `server_keys::admin_name_line`, `server_keys::admin_name_written`) | Know about agent tokens or channel access | `testing`: `testing::MIN_ACCESS_KEY_BYTES`, the shortest access key, for other crates' tests |
| `kkt-admission-os` | Server key-file texts, private key writes, admin-name text writes and staged-key moves | Choose admission rules, select a randomness provider, or word each program's key refusals | `real`: `KeyFiles`, `ReadError`, `make_default_key`, `new_key_file`, `write_key_file`, `write_admin_name` |
| `kkt-hosts` | Which host names the server answers to; whether the server may listen on an address (`check_bind`); the default address and whether any address may be used (`DEFAULT_BIND`, `DEFAULT_ALLOW_ANY_BIND`) | | none |
| `kkt-operate` | The operator's commands, among them the install's lifecycle (`lifecycle`: status, upgrade, uninstall and backup, writing backups through `BackupWriter`), setup record and operator key selection, with every effect and lazy serving question supplied through `OperateParts` | Choose native implementations; depend on another assembly or an effect crate; parse command-line flags | none |
| `kkt-server` | Assembling the server from its parts, around the `Arc<dyn AdminApi>` the program gives it (`Parts`); the settings of what it builds (`ServerSettings`, with an opaque `CheckedSettings` snapshot consumed by `Parts::new`), ordering owner checks and run-record lines with the program's plain values (`SettingError`); the server's adapter to the web page's gate (public, for a product that builds its own page: `AdmissionGate` asks kkt-admission and `Identify`); building the page's `Sessions` and handing it the rules as `OperatorApi` when it mounts the page; the web page alone over any `OperatorApi` and gate (`page`), for a program that serves only the page | Make decisions: it only wires parts together. It does not choose who a caller is: the program passes `Identify` in through `Parts` and the admission modes of `/mcp`, the web page and `/mcp-admin` through `AppConfig` (`/mcp-admin`'s is an `AdminAdmission`, which has no open mode) | none |
| `kkt-server-listen` | Binding and serving the server's assembled `App` | Decide how the server is assembled | `real`: `listen` and `Listening` |
| `kkt-config-file` | The config file's format: `parse_file`, which turns its TOML into `Config`, what the file says in its own terms (log levels as the caller's check reads them); `with_allowed_host`, which adds a host to its text; `ConfigError`; where the config file is: `config_dir` and `config_file`, under a supplied config-home folder, with their names (`CONFIG_DIR`, `CONFIG_FILE_NAME`) | Read or write files; know kkt-logs or another crate's settings: each program turns a `Config` into its own settings | none |
| `kkt-config-file-os` | The config file's bytes and the instructions file's: `read`, `read_default`, `read_named`, `read_instructions` and `write`, each whole | Know the files' format | `real`: all of it |
| `kkt-provenance` | What produced a server run, and the build script that records the git commit; the run's settings come as lines of a name and a value (`RunSettings`), which the program words | Store the run record: kkt-rules' `start` does; know another crate's settings type | none |
| `kkt-clock` | Time: the `Clock` contract, kkt's `Time` and `Span` values, and time formatting. The only crate that uses chrono | Let a chrono type into its public surface; read real time | `testing`: `FixedClock` |
| `kkt-clock-system` | The real wall clock and its sleep, implementing the shared `Clock` contract | Define time values or formatting | `real`: `SystemClock`; turns on tokio's `time` |
| `kkt-names` | Handles, IDs, the name packs, and the operator's and admins' names | Draw randomness except through `Entropy` | none |
| `kkt-entropy` | Randomness, as a contract: `Entropy`, with `random_chars`, which draws from the alphabet its caller gives | | `testing`: `SeqEntropy`, `SameHandleEntropy`, `SeededEntropy` |
| `kkt-entropy-os` | Randomness from the operating system | | `real`: `OsEntropy` (turns on `rand`) |
| `kkt-machine` | Facts about the machine: `Env` (the home folder and the folders the environment names, with the XDG ones counted only when absolute, and `config_home`, the base of per-user config that other programs' config shares too) and `Platform`, as plain values. The crates that place files take these facts and build their own paths | Read the environment; hold any of kkt's paths | none |
| `kkt-machine-os` | The machine's facts as this process sees them: `env` reads the environment and `platform` gives the platform this program was built for. The program asks once at startup and passes its `Env` and `Platform` down | | `real`: all of it |
| `kkt-events` | Notices of new messages, agents, channels and members, and of hidden agents and channels: one bus for the server, each notice heard only in its swarm, with a buffer per swarm kept only while the swarm has a subscriber | Let a tokio type out of the crate | none |
| `kkt-shutdown` | The shared stop signal, a concrete in-memory value | Let a tokio-util type out of the crate | none |
| `kkt-shutdown-os` | The operating system's stop signals, stopping the shared `Shutdown` value | Let a tokio signal type out of the crate | `real`: `StopSignals` |
| `kkt-log` | The shared structured log contract: levels, targets, messages and named values | Know a logging library or perform I/O | `testing`: `CapturedLog`, with the `Entry` records it keeps |
| `kkt-terminal-os` | The terminal's output, input and terminal detection, the closed-output-pipe default, and the terminal's half of the login link (`BrowserLink`: showing it, and the Enter key) | Decide setup answers or depend on setup/container logic | `real`: `Terminal`, `BrowserLink`, `end_on_closed_output` |
| `kkt-browser` | Opening a link in the person's browser. The contract: `Browser` (start opening it now; say later, once, if that failed). Its one implementation, `SystemBrowser`, with the system's own opener (`open` on macOS, `xdg-open` elsewhere): starting it, reaping it on a thread, and keeping a failure to report once | Name an opener program in the contract; wait for the browser; repeat the link in a message | `real`: `SystemBrowser`, which starts the opener (its choice and the failure wording are tested without it). `testing`: `Recorder` |
| `kkt-logs` | Log levels and their defaults (`DEFAULT_LOG_LEVEL`, `DEFAULT_DEPENDENCY_LOG_LEVEL`), policy checks, rollover and age-pruning rules, as pure plans | Touch the machine | none |
| `kkt-logs-os` | Writing the log files, starting the tracing subscriber, and `TracingLog` over the shared log contract | Choose rollover or pruning rules; read a real clock itself | `real`: `RotatingFile`, `init_logging`, `init_stderr_logging`, `TracingLog` |
| `kkt-setup` | `kkt setup`: what this machine is set up as, the setup record, and what follows from it: whether `kkt serve` may run here, which server this machine's clients use (`kkt switch`), and what status, upgrade, lanes, backup, key files and `kkt doctor` do; where kkt's data goes (`data_dir`, `default_db`), from kkt-machine's `Env` and `Platform`; `ask::Prompter` applies the question rules over caller-supplied `Lines`; `credential_file_check` decides credential ownership and permission tightening; doctor checks, status text, backup folders, uninstall records, client key naming and lane reports are pure setup values | Read files or run anything directly: the program adapts native providers and carries out what is decided here | `testing`: `testing::DEFAULT_SETUP_URL` and `DEFAULT_SETUP_KEY`, the defaults setup's client questions accept |
| `kkt-setup-os` | The setup record and unfinished note, data folder, program paths and PATH checks on this machine | Decide what setup does or where its files belong | `real`: all of it |
| `kkt-container` | The container installation contract and records, config parser and login link, and Docker/Compose implementation with private runner and fake | Let Docker details into the open contract; decide setup answers or remember the setup record | `testing`: `Memory`; `real`: `Docker`, with implementation-only dependencies |
| `kkt-service` | The project's `Service` contract for running kkt as a service of the user's login (install, check it started, start, stop, status, logs, add a key, uninstall), with its values (`Spec`, `ServiceState`, `Settings`) and errors (`ServiceError`); `Unsupported`, for a system with no service manager kkt works with; the plan of what the service runs, from the settings it needs of the flags and the config file (`ServiceSettings`), and whether its server could listen on its address (`check_bind`, from the program's try), both on plain values | Know a service manager, its commands or file formats; touch the machine; check the server's settings: the program does | `testing`: `Memory`, a service in memory |
| `kkt-service-launchd` | Implements `Service` directly as a launchd user agent on macOS: the plist, `launchctl`'s commands and answers, the log file and the retries while launchd removes an old service, with a private system for its own tests | Decide what the service runs | `real`: `Launchd`, the real files and commands |
| `kkt-service-systemd` | Implements `Service` directly as a systemd user unit on Linux: the unit file, `systemctl`'s and `journalctl`'s commands and answers, with a private system for its own tests | Decide what the service runs | `real`: `Systemd`, the real files and commands |
| `kkt-service-os` | Trying whether the service's server could listen on its address (`try_bind`) | Decide what the answer means | `real`: `try_bind` |
| `kkt-remote-access` | The `RemoteAccess` contract and `Node`, its testing stand-in, and Tailscale implementation with private runner and fake | Let tool details into the open contract; change login, DNS preferences, unrelated sharing, config or the service | `testing`: `Memory`; `real`: `Tailscale`, with optional parsing dependencies |
| `kkt-client-config` | Client MCP configs and their file formats; when a client's settings are changed through its own command (Claude Code), asking kkt-client-command's `ClientCommand` and restoring the file if it fails | Touch the machine except through `Host` and `ClientCommand` | none |
| `kkt-client-config-os` | The machine's client-config files, as plain `OsHost` methods | Decide a client config's content | `real`: `OsHost` |
| `kkt-client-command` | Configuring a client through its own command line: the `ClientCommand` contract in the open (remove kkt's server, add it as the client's own entry), and its one implementation, `Claude`, behind `real`: `claude mcp remove` and `add-json` at user scope, `CLAUDE_CONFIG_DIR`, and their wording, tested over a private runner | Echo the command's output, which may hold credentials | `testing`: `Recorder`. `real`: `Claude`, running the host's `claude` |
| `kkt-diagnostics` | `kkt doctor`'s checks and their plan; setup's wait for a new server | Change anything; touch the machine except through kkt-probe's `Prober` | none |
| `kkt-probe` | Probing a kkt server and reading its key files: the `Prober` contract in the open, with `KeyFile`, `Check` and the prober's `Settings`, and its one implementation, `HttpProber`, behind `real`: the HTTP probe and MCP handshake with reqwest, and key files read from disk | Let reqwest types out of the crate; choose the checks' names, timeout or protocol version: the program passes them | `testing`: `Memory`, key files and answers in memory. `real`: `HttpProber` (turns on `reqwest` and `serde_json`) |
| `kkt-backup` | Backup names, contents, modes and pruning choices, as immutable layouts; retention `Settings`, its default and bounds (`Problem`, `Settings::problems`), and its run-record line (`Settings::record_lines`) | Read the clock or copy the database itself: the caller passes both | none |
| `kkt-backup-os` | Writing backup layouts and removing selected old folders | Choose backup names, contents, modes or retention | `real`: `write` and `prune` |
| `kkt-lane-files` | Lanes as files: the `<lane>.md` format, and reading and writing a folder of them, with the commit it is at from `LaneCommit` | Touch the machine except through `Disk`; know a version-control tool | none |
| `kkt-lane-files-os` | The machine's files for lane folders, as plain `OsDisk` methods; the program adapts it to `kkt-lane-files` | Decide which lanes to import or export | `real`: `OsDisk` |
| `kkt-lane-commit` | The commit a lane folder is at: the `LaneCommit` contract in the open, and its one implementation, `Git`, behind `real`: git's commands, what their answers mean (expected absences, the dirty suffix) and its failures' wording, tested over a private runner | Read or write the lane files | `testing`: `Memory`, commits in memory. `real`: `Git`, running the machine's `git` |
| `kkt-watch` | `kkt watch` | Reach the server, the time, its place or its output except through what is injected (the server as kkt-agent-client's `Watched`) | none |
| `kkt-agent-client` | A client of kkt's server. The contract, in kkt's terms: `Servers`, which makes `Avatars` (register, reclaim, describe) and `Watched` (an agent's or the admin's view: handle, channels, latest ID, a message, a wait) for an `Address`; their records and `Failure`; what the watch asks for at a time (`Settings`). Its one implementation, over MCP through rmcp and reqwest (`Mcp`, `Client`, `Remote`, `AdminRemote`), and plain HTTP requests for a program's test client (`Http`, `HttpReply`) | Name a transport, a protocol library or its errors in the contract; let an rmcp or reqwest type out of the crate | `real`: the implementation, everything that reaches the server (turns on `reqwest`, `rmcp`, `serde_json` and `tokio`). `testing`: `Memory`, a server in memory |
| `kkt-agent-files-os` | Agent credential files and `kkt watch`'s state files on this machine: reserving and saving a token or recovery code, reading a token file under the caller's credential-file rule, file locks, claiming a state file | Choose the credential-file rule or where its warnings go | `real`: all of it (turns on `libc`) |
| `kkt-agent-commands` | The agents' commands: `kkt register` and `kkt reclaim` (with `NewAvatar`, and the wording of what the server refuses), `kkt watch` (`watch::run`, `WatchTask` and its timed-out exit code) and `kkt connect`; how a client reaches the server or the local admin endpoint, through kkt-setup's `ConnectionSource`; `AgentParts`, the effects and facts the program passes in, as its own contracts (`AdminKey`, `Credentials`, `KeyText`, `Program`, `Output`) and kkt-agent-client's (`Servers`), kkt-watch's and kkt-client-config's | Depend on an `-os` crate, on a contract's implementation, turn on another crate's `real`, or depend on another assembly; touch the machine except through `AgentParts` | none |
| `kkt-hash` | Hashing: `sha256`, the SHA-256 digest of some bytes; the one crate that uses sha2 | | none |
| `kkt-scratch` | Tests' scratch folders under `target/tmp` | Be anything but a dev-dependency | none |
| `kkt-api-sim` | A simulated application, for tests and the web preview: `Preview`, the kkt-api crates' operations the web page uses over a world made up from a seed, answering `limits` with its own values, and `PreviewGate`, a stand-in for kkt-web's `Gate`, open or with an access key checked as kkt-admission checks one | Be in a release | `testing`: all of it |

The program, in the root package:

| Part | Owns | Must not | Features |
|---|---|---|---|
| `src/bin/kkt/` (`main.rs`, `cli.rs`) | The `kkt` command: subcommands and the files and environment they read; `cli.rs` wraps clap and holds every clap type; `main.rs` builds kkt-operate's and kkt-agent-commands' parts in one place (`NativeOperate`, `agent_parts`) and runs each command through them | Pass a clap type out of `cli.rs` | Turns on `real` in every crate that has it |
| `src/machine/` (in the library `src/lib.rs`) | What kkt keeps on this machine: serve settings (`settings`: `ServeOptions`, `ServeSettings` and `server_settings`, from the flags, the config file and each owner's defaults) and the config file, the setup record, key files, how a client reaches the server (`connection::ConnectionReads`, kkt-setup's connection question answered here), the server's key reads (`keys`), this platform's service manager (`service::for_this_os`, picking kkt-service-launchd, kkt-service-systemd or kkt-service's `Unsupported`), and the adapters the two assemblies take: kkt-operate's `OperateParts` (`operate::NativeOperate`, with the serving facts its questions ask) and kkt-agent-commands' `AgentParts` (`agent_parts`, which injects kkt-agent-client's `Mcp` as it is), among them kkt-setup's credential-file rule over kkt-agent-files-os' credential and watch-state files. The setup record and the operator's key finders are kkt-operate's | | |
| `src/commands/` (in the library) | `serve`, which composes the server over kkt-server (with the backup before an upgrade), and the adapters two assemblies' commands take: `lanes`' disk and git (`LaneDisk`), and `watch`'s output (`WatchOutput`). The operator's commands are kkt-operate's, and `kkt register`, `reclaim`, `watch` and `connect` are kkt-agent-commands'; the program's tests of them over its real parts sit here and in `src/machine/` | Take a clap type | |
| `src/terminal/`, `src/process.rs` (in the library) | Adapting Terminal to setup's Lines and the terminal's BrowserLink to the container's LoginLink; this process's program path and writing a file whole (kkt-machine-os reads its environment) | | |

A second program, `programs/kkt-keymode` (package `kkt-keymode`), assembles key mode again from the shared crates with none of the `kkt` program's code: the same config file through kkt-config-file-os and kkt-config-file, into settings of its own, the same key files through kkt-admission-os and kkt-admission's checks, and kkt-server with kkt-identity-keymode's `KeyMode`. It is for development only and never released (`publish = false`, `dist = false`). It makes no key, backup or log file and adopts no old database. The HTTP-only end-to-end tests (e2e, admission, web_login, admin, watch, latency) run against it and against `kkt serve` in CI: `KKT_TEST_PROGRAM=kkt` or `kkt-keymode` makes `tests/common`'s `start` and `start_with` run that program in a process of its own (build kkt-keymode first). Unset, they run the server in the test's process. A change to `kkt serve`'s settings or startup that the tests see belongs in both programs.

The fake product, `programs/kkt-fake-product` (package `kkt-fake-product`), is the shared crates assembled as a product other than kkt would assemble them, with two swarms, `sw_main` and `sw_other`. Callers are let in by any bearer token (`Admission::Bearer`), and its `FakeIdentify` decides who each made-up bearer is: one swarm's agents, its operator on the web page, or its admin at `/mcp-admin` (`AdminAdmission::Bearer`). Its tests (`tests/inventory.rs`, `tests/slow_subscriber.rs`, `tests/refusals.rs`) start it in the test's process with `kkt_server_listen::listen` and use it over HTTP. They seed both swarms, read everything each swarm's agents, operator and admin can read, and check that nothing of the other swarm appears, and that each swarm's reads are the same byte for byte whether or not the other swarm was seeded. They also check that a slow subscriber in one swarm does not lag while the other floods. An unknown bearer, or one used for the wrong role, gets nothing on `/mcp` or the web page. Its HTTP/SSE caller lives only in `tests/client/`, through kkt-agent-client's `Http`; the library has no HTTP client. Its program serves it over a new database file through `SqlitePlace::create` and writes through `Terminal`, to look at by hand. It is dev-only and never released, like kkt-keymode.

The web preview, `programs/kkt-web-preview` (package `kkt-web-preview`), serves kkt-web's page alone (`kkt_server::page`) over kkt-api-sim's simulated application and stand-in gate, with the real clock, randomness and stop signals, so the page can be worked on with no database, MCP or agents (`docs/development.md`). It turns on kkt-api-sim's `testing`, as a dev-only program may. It is dev-only and never released.

Other files: `docs/architecture/` (the interactive crate map; run its `generate.py` after a change to crates, dependencies or public items. CI checks it is current only on pull requests into `main`, so before a release, regenerate it on `next`); `tests/migrations/sql/` (the SQL migrations kkt ran up to 0.10.0 and `next`'s swarms, which it no longer runs: the tests build older databases from them);
`crates/kkt-mcp/agent-instructions.md` and `crates/kkt-mcp/admin-instructions.md` (what agents are told
when they connect); `crates/kkt-mcp/agent-tools.toml` and `crates/kkt-mcp/admin-tools.toml` (what agents
are told about each tool); `crates/kkt-mcp/build.rs` (builds the tool text); `tests/contract/` (the tools agents see, pinned);
`CHANGELOG.md` (see `docs/releases.md`).

## Rules

**Storage.** Three crates. `kkt-db` is the contract: a narrow language of
questions and changes, with search, constraints and migrations; it knows
neither the core's tables nor any database. kkt-rules' storage module
(`crates/kkt-rules/src/store/`) knows the core's tables, declares them
and asks and changes them in kkt-db's terms, through `Reader` and
`Writer`; it knows no
database, holds no SQL and is handed a `kkt_db::Backend`. `kkt-db-sqlite` knows SQLite and no product's tables;
it is the only crate that imports rusqlite, apart from kkt-db's `testing`
stand-in (`FastPlace`, its own translation, on only in tests:
`check-release-packages.sh` keeps every `testing` out of a release) and the
program's tests that inspect database files. The program opens the file through it and
adopts the databases from before kkt-db that `kkt_rules::legacy`
describes (`src/machine/database.rs`). A
question kkt-db cannot ask is a reason to add an operation to kkt-db, with
its conformance check, never to reach past it. Rules ask for what they
need through named methods on `Reader` or `Writer`. A rule that must be
atomic runs as one `Db::write`: a write runs as if alone. Schema changes
are new kkt-db migrations at the end of `crates/kkt-rules/src/store/schema.rs`;
migrations only add, and never edit one that has shipped. Every backend
passes `kkt_db::testing::conformance`.

**Swarms.** Avatars and channels belong to a swarm; a server in key mode
has one, `sw_main`. Every store method over avatars, channels, memberships,
messages, notes or memory takes the swarm (or a `ChannelScope`, which
carries it), and every question or change in it is bound to the swarm: to a swarm,
another swarm's rows do not exist. `Rules` gets its swarm through `Deps`
and passes it on through `Bound`; `Rules::in_swarm` gives the same rules in
another swarm, sharing the database, bus, waits and rate limiters. A
subscription (`Views::subscribe`, as the operator) hears only its swarm's notices, and a lag
is reported with no count. Lanes, the server announcement, server
runs, handles and channel IDs belong to the server. A new store method over
swarm data takes the swarm and joins kkt-rules' swarms check (`testing::swarms_stay_apart_in`).

**Permissions.** Every check on whether an agent or the human may use a
channel goes through `kkt_rules::access::decide`. A store query that covers
many channels (listing, search, activity, waits) takes a `ChannelScope`
that `access::scope` builds from `decide`; the store only turns it into a
kkt-db condition and never writes its own channel condition. The test
`the_stores_channel_filters_agree_with_decide` in
`crates/kkt-rules/src/tests.rs` checks that translation against `decide`;
a new query that takes a scope joins it. To an agent outside a closed
channel, the channel does not exist: the answer is `not_found`, exactly as
for a missing ID.

**No user in the shared crates.** The crates under `crates/` are shared:
kkt-pro builds on them as kkt does. They know swarms, avatars and the
operator, never a user: no shared crate stores, names or looks up a person,
an org or a sign-in identity. Who a caller is stays with the product, in
the `Identify` and the admission mode its program passes to kkt-server; a
product with users keeps them in its own crates and tables. The fake
product's tests hold the shared crates to this.

**Dev-only packages.** A package marked `[package.metadata.kkt] dev-only =
true` (`kkt-scratch`, `kkt-api-sim`, `kkt-keymode`, `kkt-fake-product`, `kkt-web-preview`) is never part of a release:
`.github/scripts/check-release-packages.sh` fails if one is in the normal
or build dependencies of a released program (one with a binary target and no
`dist = false`), and if any workspace package's `testing` feature is on
there.

**The operator.** The operator, the human on the web page or an admin
agent at `/mcp-admin`, is an actor in `rules` (`Actor::Operator`), beside
avatars. A change the operator makes goes through the same rule as an
avatar's, and a change to a channel posts a message in it from the
operator saying what changed, in the same `Db::write`. Rules check an
`Operator`'s permissions (`Read`, `Change`, `IssueRecoveryCodes`), never
which surface asked; the edge that admits the caller gives it its
permissions and its `Audit` record, always through
`kkt_identity::Identify`, which also names the swarm a call acts in: the
two endpoints ask it for each call, and the web page's gate for each
request, wrapping its answer as the viewer. Words for the operator in a notice
come from the `OperatorWording` the program passes in `Startup` to `kkt_rules::start`, which puts it in `Deps::wording`: kkt-rules' `DefaultWording`, or a product's own, and advice in a refusal
(`RuleError::refusal`) is worded by each edge.

**What agents see.** The MCP tools are a contract with every agent. Do not
rename or remove tools, parameters, error codes or output fields; add new
ones. Tool and parameter text lives in `crates/kkt-mcp/agent-tools.toml` and
`crates/kkt-mcp/admin-tools.toml`, never in Rust, with limits written as placeholders such
as `{max_limit}`. After an intended change, run
`UPDATE_CONTRACT=1 cargo test --test contract` and review the diff to
`tests/contract/`. Keep `crates/kkt-mcp/agent-instructions.md` in step with the tools.

**kkt's names for the principles.**
- Time comes only from the injected `Clock` (clippy and the crate boundaries enforce this), randomness only from `Entropy`.
- Dependencies are injected through `kkt_rules::Deps` (`kkt_rules::Startup` at startup, where `start` records the run and builds the `Rules`), `kkt_server::Parts`, `kkt_operate::OperateParts` or `kkt_agent_commands::AgentParts`.
- Settings and defaults live with the crate that owns them: each crate
  declares the settings it takes, with their defaults and checks, and there
  is no shared configuration crate. Each program gathers a server's
  settings from flags, the config file and those defaults (kkt's in
  `src/machine/settings.rs`), and hands each crate its own. Every setting is checked at startup. Settings,
  which can differ from one server run to the next, appear in the run
  record, with each owner wording its lines and `kkt_server::ServerSettings`
  ordering them alongside the program's plain lines. Fixed constants do not: the run's recorded
  commit fixes them. Bad settings stop the server with an error naming the
  setting.
- Rules return `RuleError` with a fixed code from `crates/kkt-api-agent/src/error.rs`.
  `unwrap` and `expect` are for tests only.
- Logging happens only in side-effect and composition code (RUST.md 10). A
  logic crate that must log takes the shared log contract (`kkt-log`) as an
  injected dependency; it does not use `tracing` itself.
- The program does no I/O itself (ARCHITECTURE.md 4.3.3). Files, the
  terminal, the environment and the log are read and written by effect
  crates, which the program calls and wires in.
- Stored rows that record activity carry the `run_id` of the server run that
  made them. The one exception: `kkt lanes import` is not a server run, so
  the lane versions and retirements it saves carry the newest recorded run,
  the server the database belongs to.
- Third-party types stay inside the crates that wrap them (RUST.md 7).
- axum is the HTTP framework of `kkt-web`, `kkt-mcp`, `kkt-server` and `kkt-server-listen` only; its types do not leave those four.

**Public surface.** Make an item public only when another crate or the program needs it.

**Third-party tools and libraries (Sam, 2026-11-12, revised for phase
13).** A third-party tool or library with one implementation is one crate
named for the scope: its contract in the open, and the implementation
behind `real`. The implementation owns its tool's logic, tested against a
private runner without `real`. Several implementations, or a contract many
crates share, keep a contract crate with named implementations, as
kkt-service and kkt-db do. Either way the program injects the
implementation, with no forwarding adapter. Language-shaping libraries
retain RUST.md rule 7's exemption. This is Sam's explicit exception to
one-kind-per-crate. Phase 12 split each tool into a contract crate and a
wrapper; the plan's phase 13 merges the pairs with one implementation.

**Dependencies between crates.** Add one only for a real reason. A crate
that depends on another for something minor, such as one small helper, is
a sign the crate lines are wrong.

## Tests

- Run `cargo test`, `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt --check` before every commit. CI runs the same on Ubuntu and
  macOS, and a pull request merges only when they pass.
- CI also builds a crate outside the workspace that depends on
  kkt-api-agent, kkt-rules, kkt-server and kkt-server-listen (with `real`), whose dependencies have
  build scripts, by git at the commit under test, as kkt-pro will
  (`.github/scripts/build-outside-crate.sh`): a shared crate must build
  when used that way.
- CI also checks two of the refactor's rules. check-crates.sh fails on a
  `real` feature, or an item behind one, in a crate concerns.json marks as a
  contract. `docs/architecture/check-programs.py` builds the map from the
  code and fails on machine work in a program's own code that the refactor
  plan's temporary breaks do not list.
- A bug fix, a robustness change or a performance claim comes with a test
  that fails without it.
- Every pull request says whether it can affect an existing install, and how
  that was checked. A schema change needs a test that a database from the
  previous release migrates: the 0.10.0 fixture (`tests/migrations/v0_10_0.rs`)
  must still answer every pinned read and end up the same as a new database.
- Unit tests sit beside their code. Rule tests in `crates/kkt-rules/src/tests.rs` use
  `FixedClock` and `SeqEntropy` for the rules' time and randomness. A test may use real time only to coordinate its own tasks, with a clippy exception and a reason. End-to-end tests in
  `tests/` start servers through `tests/common`.
- Tests that run the `kkt` binary give it its own `HOME`. Scratch files go
  under `target/tmp` through `scratch::Dir`, never the system temporary
  folder; CI makes that folder unwritable.

## Working with other agents

Stay inside your own working directory. Share work only through git: push a
branch and open a pull request with `gh`. Do not read or copy another agent's
files. Pull requests target the integration branch the coordinator names. Only
the repository owner merges into `main`.
