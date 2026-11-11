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
stay outside it. Only programs (binaries and examples) and integration tests
turn on `real`, and a contract crate never has one. Stand-ins and other test
support are behind a `testing` feature, which other crates turn on only in
their dev-dependencies (RUST.md 6); a crate's own `testing` may turn on
its dependencies' `testing`. A fake used only inside one crate stays
in that crate's unit tests. CI builds each crate on its own and checks these
rules (`.github/scripts/check-crates.sh`). A crate with no feature listed
has none.

| Crate | Owns | Must not | Features |
|---|---|---|---|
| `kkt-rules` | Every permission decision and every change, for agents and the human; the core's data, in its storage module (`src/store/`): its questions (`Reader`) and changes (`Writer`) in kkt-db's terms, and the core's tables as kkt-db migrations (`src/store/schema.rs`) | Reach the database except through the storage module's `Reader` and `Writer`; format output; contain SQL or know a particular database (it is given a `kkt_db::Backend`) | `testing`: `testing::swarms_stay_apart` (the check every store must pass) |
| `kkt-api-agent` | What any registered agent may do, and the records every level shares: agents, channels, messages, memory, lanes, the queries, who acts (`Actor`, `Agent`, `Operator`, `Audit`, `Permissions`) and the errors (`RuleError`, `Code`) | | none |
| `kkt-api-operator` | What the human who runs the server may do, by area (`Views`, `Changes`, `Lanes`, `Server`), with the records only they use; `kkt-rules` implements it | | none |
| `kkt-db` | The database contract, for any database: values; declaring tables, keys, references, indexes and constraints; the questions and changes a store may make (a narrow language, not SQL); transactions (a read sees one state, a write runs as if alone); search: what a search asks for (`Search`, `Excerpt`) and finds (`Hit`); migrations as additions, recorded per named set; `Legacy`, a database from before migrations were tracked, in no backend's terms; the `Backend` and `Evolve` traits a backend implements, and `BackendExt`, the typed read and write a store calls on any backend | Know a product's tables; name a database or use a database library | `testing`: `MemoryPlace`, a database in memory that keeps the contract; `conformance`, the checks every backend must pass; `FaultyBackend`, a backend that wraps another and fails a change on cue |
| `kkt-db-sqlite` | kkt-db over SQLite, the only crate that knows SQLite: SQL written from kkt-db's operations, one writer behind a lock and read-only readers, constraints as triggers, search indexes as FTS5 over kkt-search's words, migrations recorded in `kkt_db_versions`, adopting a database from before kkt-db as the caller's kkt-db `Legacy` describes, reading SQLite's `user_version` itself (`SqlitePlace::adopting`), `copy_database` | Know a product's tables; let a rusqlite type out of the crate | `real`: `SqlitePlace` and `copy_database`, which open database files. `testing`: `MemoryPlace`, a SQLite database in memory |
| `kkt-search` | Search as text: the words of a text (FTS5 `unicode61`'s rule, as a table of our own), a search's terms, whether a text matches them, and an excerpt around the matches, drawn with the marks and window it is given | Know a database | none |
| `kkt-mcp-formatter` | The MCP tools' results turned into text or JSON, within the response size cap; `NoticeWording`, the default words for the operator in a notice | Decide permissions; read the database | none |
| `kkt-mcp` | The agent and admin MCP tools; what the two share; the endpoints' HTTP transport; their text | Touch the database except through the rules; hold tool or parameter text in Rust; decide who a caller is except through `kkt_identity::Identify` | none |
| `kkt-identity` | Who a caller is, as a contract: `Identify`, which turns an admitted caller's `Principal` into (swarm, actor) or a refusal; the one place that decides who someone is and what permissions they get | Touch HTTP: each surface reads the principal off its own requests | `testing`: `FixedIdentify`, a stand-in `Identify` |
| `kkt-identity-keymode` | Key mode's identity: `KeyMode`, which implements `Identify` from the key that let a caller in and the avatar's token, in the one swarm | Touch HTTP | none |
| `kkt-web` | The human's pages, through its own contract (`src/source.rs`: `Gate`, which says who is viewing each request; `Sources`, which gives each viewer its `Source`; and `Source`) | Depend on the rules, the store, admission or identity: `kkt-server` adapts them to its contract | `testing`: the page scenarios every source must pass (`kkt_web::testing`); turns on `tower` and tokio's `time` |
| `kkt-admission` | Whether an HTTP caller is let in at all, and as which `Principal`: no key, which of the server's keys, or (for another program's `Identify` to judge; kkt never sets it) any bearer token; web sessions; the checks on the server's keys and what to say when one is refused (`key_refusal`); which access key a server uses (`AccessKey`); where the access key file goes by default (`access_key_file`, in the config folder it is given), and where the web key, the admin key and the admin name's file go beside it (`web_key_path`, `admin_key_path`, `admin_name_path`) | Know about agent tokens or channel access | `testing`: `testing::MIN_ACCESS_KEY_BYTES`, the shortest access key, for other crates' tests |
| `kkt-hosts` | Which host names the server answers to | | none |
| `kkt-server` | Assembling the server from its parts; recording each server run; the server's adapters to the web contract (public, for a product that builds its own page) | Make decisions: it only wires parts together. It does not choose who a caller is: the program passes `Identify` in through `Parts` and the admission modes of `/mcp`, the web page and `/mcp-admin` through `AppConfig` (`/mcp-admin`'s is an `AdminAdmission`, which has no open mode) | none |
| `kkt-server-listen` | Binding and serving the server's assembled `App` | Decide how the server is assembled | `real`: `listen` and `Listening` |
| `kkt-settings` | Gathering a server's settings from flags, what the config file gives and defaults, including the limits (`Limits`), which the program turns into each crate's own settings; the server's defaults (`DEFAULT_BIND`, the log levels); `InvalidSetting`, its error for a bad gathered value; the settings a server run records, worded as lines of a name and a value (`RecordedSettings`). Only the programs depend on it | Read files or the environment, or parse the config file: callers pass in what the file gives and the defaults that depend on the machine, such as the default database | none |
| `kkt-config-file` | The config file's format: `parse_file`, which turns its TOML into `Config`, what the file says in its own terms (log levels as the caller's check reads them); `with_allowed_host`, which adds a host to its text; `ConfigError`; where the config file is: `config_dir` and `config_file`, in kkt-machine's `Env::config_home`, with their names (`CONFIG_DIR`, `CONFIG_FILE_NAME`) | Read or write files; know kkt-settings or kkt-logs: the programs turn a `Config` into kkt-settings' `ServeOptions` | none |
| `kkt-config-file-os` | The config file's bytes and the instructions file's: `read`, `read_default`, `read_named`, `read_instructions` and `write`, each whole | Know the files' format | `real`: all of it |
| `kkt-provenance` | What produced a server run, and the build script that records the git commit; the run's settings come as lines of a name and a value (`RunSettings`), which the program words | Store the run record: `kkt-server` does; know another crate's settings type | none |
| `kkt-clock` | Time: the `Clock` contract, kkt's `Time` and `Span` values, and time formatting. The only crate that uses chrono | Let a chrono type into its public surface; read real time | `testing`: `FixedClock` |
| `kkt-clock-system` | The real wall clock and its sleep, implementing the shared `Clock` contract | Define time values or formatting | `real`: `SystemClock`; turns on tokio's `time` |
| `kkt-names` | Handles, IDs, the name packs, and the operator's and admins' names | Draw randomness except through `Entropy` | none |
| `kkt-entropy` | Randomness, as a contract: `Entropy`, with `random_chars`, which draws from the alphabet its caller gives | | `testing`: `SeqEntropy`, `SameHandleEntropy`, `SeededEntropy` |
| `kkt-entropy-os` | Randomness from the operating system | | `real`: `OsEntropy` (turns on `rand`) |
| `kkt-machine` | Facts about the machine: `Env` (the home folder and the folders the environment names, with the XDG ones counted only when absolute, and `config_home`, the base of per-user config that other programs' config shares too) and `Platform`; `Machine`, which gives them. The crates that place files take these facts and build their own paths | Read the environment; hold any of kkt's paths | `testing`: `FixedMachine`, a stand-in `Machine` |
| `kkt-machine-os` | The machine's facts as this process sees them: `OsMachine` reads the environment and the platform this program was built for. The program makes one at startup and passes its `Env` and `Platform` down | | `real`: all of it |
| `kkt-events` | Notices of new messages, agents, channels and members, and of hidden agents and channels: one bus for the server, each notice heard only in its swarm, with a buffer per swarm kept only while the swarm has a subscriber | Let a tokio type out of the crate | none |
| `kkt-shutdown` | The shared stop signal, a concrete in-memory value | Let a tokio-util type out of the crate | none |
| `kkt-shutdown-os` | The operating system's stop signals, stopping the shared `Shutdown` value | Let a tokio signal type out of the crate | `real`: `StopSignals` |
| `kkt-logs` | Log levels, policy checks, rollover and age-pruning rules, as pure plans | Touch the machine | none |
| `kkt-logs-os` | Writing the log files and starting the tracing subscriber | Choose rollover or pruning rules; read a real clock itself | `real`: `RotatingFile`, `init_logging`, `init_stderr_logging` |
| `kkt-setup` | `kkt setup`: what this machine is set up as, the setup record, and what follows from it: whether `kkt serve` may run here, which server this machine's clients use (`kkt switch`), and what status, upgrade, lanes, backup, key files and `kkt doctor` do; where kkt's data goes (`data_dir`, `default_db`), from kkt-machine's `Env` and `Platform` | Read files or run anything: the program does, and carries out what is decided here | `testing`: `testing::DEFAULT_SETUP_URL` and `DEFAULT_SETUP_KEY`, the defaults setup's client questions accept |
| `kkt-container` | Running kkt in Docker: its Compose files, its folder, and the commands that manage it; `ConfigReader`, the contract of whoever parses the container's config file for it; the defaults for a container whose config names no name pack or lane limit come from the caller | Decide what a person wants or remember what was set up; touch the machine except through `Host`; parse the config file except through `ConfigReader` | none |
| `kkt-container-os` | The machine's Docker commands, files and terminal, as plain `OsHost` methods | Decide what Docker setup does | `real`: `OsHost` |
| `kkt-service` | Running kkt as a service (launchd, systemd), the plan of what the service runs, from the settings it needs of the flags and the config file (`ServiceSettings`), and whether its server could listen on its address (`check_bind`, from the program's try) | Touch the machine except through `System`; check the server's settings: the program does | none |
| `kkt-service-os` | The machine's service files and commands, as plain `OsSystem` methods | Decide what a service command does | `real`: `OsSystem` |
| `kkt-remote-access` | Sharing a service-mode server over Tailscale | Change the Tailscale login, DNS preferences, kkt's config or its service | none |
| `kkt-remote-access-os` | The machine's `tailscale` command, as plain `OsHost` methods | Decide how the server is shared | `real`: `OsHost` |
| `kkt-client-config` | Client MCP configs and their file formats | Touch the machine except through `Host` | none |
| `kkt-client-config-os` | The machine's client-config files and `claude` command, as plain `OsHost` methods | Decide a client config's content | `real`: `OsHost` |
| `kkt-diagnostics` | `kkt doctor`'s checks and their plan; setup's wait for a new server | Change anything; touch the machine except through `Prober` | none |
| `kkt-diagnostics-os` | The machine's key files and HTTP probe, returning its own records | Let reqwest types out of the crate | `real`: `OsProber` |
| `kkt-backup` | Backup names, contents, modes and pruning choices, as immutable layouts | Read the clock or copy the database itself: the caller passes both | none |
| `kkt-backup-os` | Writing backup layouts and removing selected old folders | Choose backup names, contents, modes or retention | `real`: `write` and `prune` |
| `kkt-lane-files` | Lanes as files: the `<lane>.md` format and the folder's git commit | Touch the machine except through `Disk` and `git::Git` | none |
| `kkt-lane-files-os` | The machine's files and git for lane folders, as plain `OsDisk` methods; the program adapts it to `kkt-lane-files` | Decide which lanes to import or export | `real`: `OsDisk` |
| `kkt-watch` | `kkt watch` | Reach the server, the time, its place or its output except through what is injected | none |
| `kkt-agent-client` | The shared MCP client for agent commands and watching, returning its own records and failures | Implement kkt-watch's contract: the program adapts it | `real`: `Client`, `Remote`, `AdminRemote`, which reach the server (turns on `reqwest` and `rmcp`) |
| `kkt-hash` | Hashing: `sha256`, the SHA-256 digest of some bytes; the one crate that uses sha2 | | none |
| `kkt-scratch` | Tests' scratch folders under `target/tmp` | Be anything but a dev-dependency | none |

The program, in the root package:

| Part | Owns | Must not | Features |
|---|---|---|---|
| `src/bin/kkt/` (`main.rs`, `cli.rs`) | The `kkt` command: subcommands and the files and environment they read; `cli.rs` wraps clap and holds every clap type | Pass a clap type out of `cli.rs` | Turns on `real` in every crate that has it |
| `src/machine/` (in the library `src/lib.rs`) | What kkt keeps on this machine: serve settings and the config file, the setup record, key files, how a client reaches the server, agent credential files; `credentials.rs` wraps libc | | |
| `src/commands/` (in the library) | One module per command, each with a `run` taking plain inputs: `serve`, `service`, `setup` (with `OsSteps` and `OsSwitchSteps`, setup's real steps, in `setup/steps.rs`), `doctor`, `lifecycle` (status, upgrade, uninstall and backup, with the backup `serve` also makes), `key`, `agent` (register, reclaim and `kkt agent`), `lanes` (with the lane helpers `setup` also uses), `watch` (with `WatchOutput`) and `connect` | Take a clap type | |
| `src/terminal/`, `src/process.rs` (in the library) | Asking setup's questions and the Tailscale login link on the terminal; this process's closed-output setting, program path, and writing a file whole (kkt-machine-os reads its environment); `process.rs` wraps libc | | |

A second program, `programs/kkt-keymode` (package `kkt-keymode`), assembles key mode again from the shared crates with none of the `kkt` program's code: the same config file through kkt-config-file-os, kkt-config-file and kkt-settings, the same key files through kkt-admission, and kkt-server with kkt-identity-keymode's `KeyMode`. It is for development only and never released (`publish = false`, `dist = false`). It makes no key, backup or log file and adopts no old database. The HTTP-only end-to-end tests (e2e, admission, web_login, admin, watch, latency) run against it and against `kkt serve` in CI: `KKT_TEST_PROGRAM=kkt` or `kkt-keymode` makes `tests/common`'s `start` and `start_with` run that program in a process of its own (build kkt-keymode first). Unset, they run the server in the test's process. A change to `kkt serve`'s settings or startup that the tests see belongs in both programs.

The fake product, `programs/kkt-fake-product` (package `kkt-fake-product`), is the shared crates assembled as a product other than kkt would assemble them, with two swarms, `sw_main` and `sw_other`. Callers are let in by any bearer token (`Admission::Bearer`), and its `FakeIdentify` decides who each made-up bearer is: one swarm's agents, its operator on the web page, or its admin at `/mcp-admin` (`AdminAdmission::Bearer`). Its tests (`tests/inventory.rs`, `tests/slow_subscriber.rs`, `tests/refusals.rs`) start it in the test's process with `kkt_server_listen::listen` and use it over HTTP. They seed both swarms, read everything each swarm's agents, operator and admin can read, and check that nothing of the other swarm appears, and that each swarm's reads are the same byte for byte whether or not the other swarm was seeded. They also check that a slow subscriber in one swarm does not lag while the other floods. An unknown bearer, or one used for the wrong role, gets nothing on `/mcp` or the web page. Its program serves it over a new database file, to look at by hand. It is dev-only and never released, like kkt-keymode.

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
it is the only crate that imports rusqlite, apart from the program's tests
that inspect database files. The program opens the file through it and
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
subscription (`Rules::subscribe`) hears only its swarm's notices, and a lag
is reported with no count. Lanes, the server announcement, server
runs, handles and channel IDs belong to the server. A new store method over
swarm data takes the swarm and joins `kkt_rules::testing::swarms_stay_apart`.

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
true` (`kkt-scratch`, `kkt-keymode`, `kkt-fake-product`) is never part of a release:
`.github/scripts/check-release-packages.sh` fails if one is in the normal
or build dependencies of a released program (one with a binary target and no
`dist = false`).

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
come from kkt-mcp-formatter through `Deps::wording`, and advice in a refusal
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
- Dependencies are injected through `kkt_rules::Deps` or `kkt_server::Parts`.
- Settings and defaults live with the crate that owns them: each crate
  declares the settings it takes, with their defaults and checks, and there
  is no shared configuration crate. `kkt-settings` gathers a server's
  settings from flags, the config file and those defaults, and the program
  hands each crate its own. Every setting is checked at startup. Settings,
  which can differ from one server run to the next, appear in the run
  record, worded by `kkt_settings::RecordedSettings`. Fixed constants do not: the run's recorded
  commit fixes them. Bad settings stop the server with an error naming the
  setting.
- Rules return `RuleError` with a fixed code from `crates/kkt-rules/src/error.rs`.
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
