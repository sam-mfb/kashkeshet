# Refactor plan

This plan turns the code on `next` into the proposed crate map. The map's
proposal (`proposed.json`, shown on the page) and `decisions.md` say what the
code should become. `principles/` and `CLAUDE.md` bind every step. The plan
says in what order to get there, in steps small enough to review.

Every step is one pull request into the `refactor` branch, cut from `next`
at a350e48. Each leaves the whole workspace building and every test passing.
Sam approves `refactor` into `next` as a whole at the end.

The plan has five parts: the rules every step follows; how progress is
measured; how parallel work avoids collisions; the steps; and what needs
Sam, each with the default we proceed on.

The refactor's phases 0 to 12 are done. After phase 8, Sam decided four
more (2026-11-12). Phase 9 removed kkt-settings: each owner checks and words
its own settings, and kkt-server takes them checked. Phase 10 moved the
program's tools out of its library into two assemblies, kkt-operate and
kkt-agent-commands, while the program keeps composing the server. Phase 11
gave kkt-db a fast test stand-in, so kkt-rules' tests no longer use an effect
crate. Phase 12 split every third-party dependency into a contract crate in
kkt's terms and a wrapper named for the tool, which the program injects
with no adapter. What is left for Sam comes first, below; the scorecards
are at the end.

## Awaiting Sam

Every row below is decided by Sam, or is on one of the two lists at the end
of this section: the differences to show him again now that phases 9 to 11
are done, and the new ones those phases made. They were the questions
left after phases 0 to 8: every difference from the proposed map then
awaiting Sam, and the open items of "What needs Sam". They are grouped by
how much they change. Each line gives the step and the answer, with the
recommendation from kkt-mt-step1, the plan's author, where Sam has not
ruled. Where a difference is a compromise, the line says so.

**(a) Changes to a rule, or to which crate may depend on which.**

| Difference | Step | Recommended |
|---|---|---|
| The shared crates' public surface changes for kkt-pro, which builds on them by git ("What needs Sam" 2) | all | **Accepted by Sam, 2026-11-12.** The CHANGELOG entry says what a dependant must change, and the outside-crate CI job still builds |
| check-crates rule 3 lets a dev-only program turn on another package's `testing` (the web preview, kkt-api-sim) ("What needs Sam" 4; the 6.3 row on kkt-web-preview) | 6.3 | **Accepted by Sam, 2026-11-12.** A dev-only package can never be in a release, and check-release-packages.sh enforces that |
| The fake product's tests turn on kkt-agent-client's `real` | 7.5; fixed after 8.1 | **Decided by Sam, 2026-11-12: SHOULD, with this documented exception.** Tests SHOULD NOT turn on `real`; check-crates.sh takes a list of documented exceptions (`.github/scripts/real-exceptions.json`: package, dependency, reason), and fails on any other and on a listed one that no longer applies. Its one entry is the fake product's tests, which drive a running server over HTTP. kkt-agent-client is a dev-dependency of the fake product again, so the arrow is a test arrow |
| A contract crate depends on a contract crate: kkt-api-admin on kkt-api-operator (`AdminApi` extends `OperatorApi`) | 5.4b | **Accepted by Sam, 2026-11-12.** An endpoint moved into a swarm needs every operation it uses there, and contracts depending on contracts adds no effect or logic |
| kkt-settings depends on kkt-admission, kkt-logs, kkt-mcp-formatter, kkt-web and kkt-names | 3.1b, 3.5b, 3.5c | **Not accepted by Sam, 2026-11-12: kkt-settings goes (phase 9).** It gathers other crates' concerns under one name, as kkt-config did. (The recommendation was to accept, as a compromise, since no cycle could form.) |
| kkt-config-file takes the config-home folder; `config_home` stays a method of `Env` | 3.4 | **Decided by Sam, 2026-11-12: kkt-config-file takes the folder, not Env.** The caller computes `env.config_home()` and passes its optional path; kkt-config-file no longer depends on kkt-machine |

**(b) Differences that leave a rule broken.** The scorecard counts the
first two.

| Difference | Step | Recommended |
|---|---|---|
| kkt-rules' tests use kkt-db-sqlite's in-memory database, an effect crate's | 5.3b | **Not accepted by Sam, 2026-11-12, and no exception: done in 11.1 and 11.2.** kkt-db's `testing` has a fast stand-in on SQLite in memory with its own translation (`FastPlace`, 11.1), and kkt-rules' tests and `testing::database` run on it (11.2, 6 s); the program's `tests/store_on_sqlite.rs` runs the store's whole surface on kkt-db-sqlite (11.2), and 11.3 adds a chosen subset of the rules' scenarios there |
| The arrow kkt-server → kkt-api-admin is missing: `Parts` holds the concrete `Rules` | 5.6a | **Decided by Sam, 2026-11-12: `Parts` takes the contract** (9.2): an `Arc<dyn AdminApi>`, which the program fills with what `kkt_rules::start` returns; kkt-server's normal dependency on kkt-rules goes. (The recommendation was to accept, and drop the arrow from the proposal.) |

**(c) Names, placements and extra items only.** Accepted by Sam,
2026-11-12, in one go, the two compromises included. None changes what a
crate may depend on or breaks a rule; each keeps a behaviour or a message as
it was, or names something the proposal left unnamed.

| Difference | Step | Sam |
|---|---|---|
| kkt-lane-files declares a narrow git contract | 2.6 | **Accepted by Sam, 2026-11-12**: git's meaning stays in logic. Overtaken by 12.5a (Sam's ruling, msg 3041): git's meaning is kkt-lane-commit-git's |
| One watch adapter, an enum over `Remote` and `AdminRemote` | 1.9 | **Accepted by Sam, 2026-11-12**: fewer methods, same arrows. Overtaken by 12.4: the adapter is gone |
| kkt-service-os reports with its own `Finished` | 2.2 | **Accepted by Sam, 2026-11-12**: the effect depends on nothing. Overtaken by 12.3: the launchd and systemd wrappers implement `Service` directly |
| kkt-remote-access-os returns its own output and error | 2.3 | **Accepted by Sam, 2026-11-12**: the effect depends on nothing. Overtaken by 12.1: kkt-remote-access-tailscale implements `RemoteAccess` directly |
| kkt-client-config-os states its own file mode | 2.4 | **Accepted by Sam, 2026-11-12**: the same fixed value, kept with its writer |
| kkt-container-os takes and returns its own values, and states its own file mode | 2.1 | **Accepted by Sam, 2026-11-12**: the effect depends on nothing. Overtaken by 12.2: kkt-container-docker implements `Container` directly |
| Each crate's own `Settings` with a `DEFAULT`, in place of item 7's constants | 3.1a, 3.1b, 3.1c-1 | **Accepted by Sam, 2026-11-12**: it is how a crate takes injected values |
| kkt-diagnostics' two public doctor constants; kkt-agent-client's `Settings` outside `real` | 3.1c-1 | **Accepted by Sam, 2026-11-12**: the effect chooses no values |
| Backup logic gives a fixed layout for backup-os to write | 2.7 | **Accepted by Sam, 2026-11-12**: the decisions stay in logic |
| kkt-config-file-os has five readers and writers, not three | 3.3 | **Accepted by Sam, 2026-11-12**: each keeps its caller's behaviour and messages |
| `parse_file` takes the caller's log-level check; `ConfigReader` returns its own `ContainerConfig` | 3.3 | **Accepted by Sam, 2026-11-12**: kkt-config-file need not know kkt-logs |
| kkt-logs gives rollover and pruning plans, which kkt-logs-os carries out | 4.1 | **Accepted by Sam, 2026-11-12**: the rules stay in logic |
| kkt-logs' plans' names; `LOG_PRUNE_INTERVAL` stays private; kkt-logs' own `SettingError` | 3.1c-2, 4.1 | **Accepted by Sam, 2026-11-12**: names for what the plans do |
| The log contract's field records, `Level::Trace` and `testing::Entry` | 4.2 | **Accepted by Sam, 2026-11-12**: keeps every level and field without tracing's types |
| `Audit::AdminEndpoint` holds the admin name as text, not `AdminName` | 5.3a | **Accepted by Sam, 2026-11-12**, as a compromise: the contract depends on no kkt crate, at the cost of a checked type |
| kkt-api-agent holds `LaneSource` | 5.3a | **Accepted by Sam, 2026-11-12**: a shared record holds it |
| kkt-api-operator holds `Overview`, `ChannelView`, `LaneHistory` and their parts | 5.4a | **Accepted by Sam, 2026-11-12**: a contract owns what it returns |
| The operations return kkt-api-agent's `Answer`, a boxed future | 5.4a, 5.3b | **Accepted by Sam, 2026-11-12**: the traits work through `dyn` |
| `Terminal` adds `flush_output`, `message` and `output_is_terminal` | 7.1a | **Accepted by Sam, 2026-11-12**: keeps prompt and watch output as they were |
| `Terminal::interactive` checks input, `output_is_terminal` output | 7.1b | **Accepted by Sam, 2026-11-12**: setup asks as it did before |
| The key-file helpers take a key maker and keep each read error's path and cause | 7.2 | **Accepted by Sam, 2026-11-12**: each program's refusal messages stay the same |
| kkt-mcp-formatter no longer depends on kkt-rules | 5.5 | **Accepted by Sam, 2026-11-12**: one arrow fewer |
| kkt-web's `Sessions<T>`, `Gate::admits` and `Gate::sign_in` | 6.1 | **Accepted by Sam, 2026-11-12**: the page keeps its sessions |
| kkt-admission's `WebAdmission` decides over its own `WebSession` | 6.1 | **Accepted by Sam, 2026-11-12**: the access decision and its texts stay in kkt-admission |
| `Views::subscribe` gives the operator's `Notices`, with records of its own | 6.2 | **Accepted by Sam, 2026-11-12**: a contract may not name kkt-events, and the live feed is unchanged |
| kkt-web's `Gate` admits kkt-web's `Viewer` record; `WebViewer` goes | 6.2 | **Accepted by Sam, 2026-11-12**: the page must read the swarm and the operator |
| kkt-web's `Settings` holds `PageLimits` | 6.2 | **Accepted by Sam, 2026-11-12**: one settings value per crate |
| The preview keeps its own records and translates them | 6.2, 6.3 | **Accepted by Sam, 2026-11-12**, as a compromise: more code in kkt-api-sim, in place of rewriting the simulation |
| kkt-web's test scenarios take an `App` in place of a `Source` | 6.2 | **Accepted by Sam, 2026-11-12**: the scenarios read the application |
| The agent credential readers' checks, facts and callbacks | 7.4 | **Accepted by Sam, 2026-11-12**: kkt-agent-client stays free of setup and the terminal |
| kkt-web-preview is a program only, and the preview's tests are kkt-api-sim's | 6.3 | **Accepted by Sam, 2026-11-12**: tests sit beside the stand-in they test (its `testing` is in group (a)) |
| kkt-server's `page`; the preview depends on kkt-server and kkt-server-listen | 6.3 | **Accepted by Sam, 2026-11-12**: axum stays in its four crates, and the program binds no socket itself |
| The preview logs through kkt-logs-os and reads its key through kkt-admission-os | 6.3 | **Accepted by Sam, 2026-11-12**: the program does no machine work itself |
| kkt-api-sim's `PreviewSettings` and `SettingError`; the page size is the preview's flag | 6.3 | **Accepted by Sam, 2026-11-12**: each crate declares its own settings |
| kkt-api-sim depends on kkt-shutdown; kkt-web no longer on kkt-events | 6.3 | **Accepted by Sam, 2026-11-12**: the stop is the caller's |
| kkt-mcp's tests do not use kkt-api-sim | 6.3 | **Accepted by Sam, 2026-11-12**: they need nothing it gives |
| kkt-server's and kkt-web's tests use kkt-api-sim and kkt-shutdown | 6.3 | **Accepted by Sam, 2026-11-12**: test arrows to stand-ins |
| `Http` returns an opaque `HttpReply`; `SqlitePlace::create` reports `CreateError` | 7.5 | **Accepted by Sam, 2026-11-12**: no library type leaves its crate |
| Setup's connection records, path callback and lazy readers | 7.3a | **Accepted by Sam, 2026-11-12**: keeps the first read and its error |
| kkt-setup-os' path and note functions; kkt-container's `Host` (a trait) and kkt-admission-os' `KeyFiles` (a native provider) expose `is_file` | 7.3a | **Accepted by Sam, 2026-11-12**: path work stays in effect crates |
| `docker_env` returns the read refusal, not an absent value | 7.3a | **Accepted by Sam, 2026-11-12**: upgrade refuses a missing `.env` as before |
| Setup returns plain doctor and lane-import values | 7.3b | **Accepted by Sam, 2026-11-12**: setup depends on no implementation crate |
| Setup's status, backup, uninstall, key-naming and lane-limit functions | 7.3b | **Accepted by Sam, 2026-11-12**: decisions move to their owner, messages unchanged |
| `bound_to` and `bound_in` are free functions; `admin_name_line` takes a reader; `admin_name_written` | 7.7 | **Accepted by Sam, 2026-11-12**: a crate cannot add methods to another's type |

**(d) The merge, from "What needs Sam".**

| Question | Item | Sam |
|---|---|---|
| Remove `refactor` from CI's branch lists | 1 | **Accepted by Sam, 2026-11-12.** In the pull request that merges `refactor` into `next` (`refactor-merge.md`) |
| Log output | 5 | **Accepted by Sam, 2026-11-12.** The same events, targets, levels and fields reach the log |
| How `refactor` merges into `next` | 6 | **Accepted by Sam, 2026-11-12.** One merge commit, no force-push, after a final review of the whole diff and a full CI run |
| macOS in CI | 7 | **Accepted by Sam, 2026-11-12, and done sooner:** the macOS checks came back on `refactor` (#486), then went off again the same day, since GitHub's macOS runners are short (see "Temporary breaks") |
| The proposed map after the refactor | 8 | **Accepted by Sam, 2026-11-12.** The proposal is kept as a record, and "Current" is the code |

**To show Sam again, now that phases 9 to 11 are done.** Sam's decisions of
2026-11-12 overtook these differences. Each step that replaced one has
merged; the last column says how it ended.

| Difference | Step | How it ended |
|---|---|---|
| `serve_bind` stays in the program; kkt-service's `check_bind`; kkt-keymode keeps four refusal messages | 3.5b | 9.3 (#495): the serve surface is each program's. kkt's `serve_bind` is in its `machine::settings`, the service's bind check is still kkt-service's `check_bind`, and kkt-keymode keeps its own surface and messages. There is no kkt-serve |
| No `SettingsSource`: the programs turn `Config` into `ServeOptions` | 3.3, 3.5c | 9.3 (#495): kkt-settings is gone. Each program turns kkt-config-file's `Config` and its flags into its own `ServeOptions` |
| Each program keeps `check_settings`, about 25 lines in both kkt and kkt-keymode | 3.5c | 9.1 (#484, #485), 9.2 (#494): each owner checks its own settings and reports a `Problem`; kkt-server's `ServerSettings::check` asks them in today's order and returns a `CheckedSettings`, which `Parts::new` takes. kkt's `check_settings` passes it kkt-rules' and kkt-backup's problems and its own (the bind and the log level), then reads the instructions and key files |
| kkt-settings words the run's settings; kkt-provenance depends only on kkt-hash | 3.5c | 9.1, 9.2 (#494): each owner words its own lines, and `ServerSettings::record_lines` orders them with the program's. kkt-provenance still depends only on kkt-hash |
| `start` takes `NewServerRun` and `Startup`; `Parts::new` takes the `Rules`; the program makes the events bus | 5.6a | 9.2 (#494): `Parts` takes an `Arc<dyn AdminApi>`, so kkt-server no longer depends on kkt-rules, and `new_server_run` is the caller's of `start` (kkt's `machine::server_run`, and kkt-keymode's own) |
| `local_web_key` and `local_operator_key` stay in the program | 7.3b | 10.2a (#496): they are kkt-operate's (`keys`), still outside kkt-setup, and ask the local server's facts through `ServeFacts` |
| `kkt_rules::testing` gives the program's tests the store's `Reader`, `Writer`, records and filters, with `read` and `write`; `testing::database` gives other tests a bare `Db` | 5.6b | 11.4 (#492): `testing` names no store type. The program's tests use purpose-named functions (`every_store_read`, `rules_on`, `swarms_stay_apart_in`, `scenarios::run_all`); `testing::database` stays |

**New differences for Sam from phases 9 to 11.** Each has a row in the
differences table below, and is charted into the proposal.

| Difference | Step | Recommended |
|---|---|---|
| `ServerSettings::check` returns an opaque `CheckedSettings`, which `Parts::new` takes, so a server cannot be built from settings never checked | 9.2 (#494) | Accept: asked for in 9.2's review |
| kkt's `new_server_run` is public, in `machine::server_run`, so the integration tests' harness records a run as `kkt serve` does | 9.2, 9.3 | Accept: one mapping, not a copy in the tests |
| `ServiceDefaults`: only operator parts built with the service-file parser can ask whether an installed service runs the defaults | 10.2a (#496) | Accept: the compiler refuses the question where no parser was given, in place of a refusal at run time |
| `ServeFacts::service_settings` gives the service plan's inputs and a deferred `CheckService`, which keeps the program's whole check until the plan is made | 10.2b (#498) | Accept: the check runs in the same order as before, and the assembly sees nothing of the program's settings |
| kkt-container's `tailscale_auth_key` and `docker_env` take a possibly unsized `Host` | 10.2b (#499) | Accept: every earlier caller still compiles |
| kkt-operate's module for `kkt agent` is `agent`, not `agents` | 10.2a (#496) | Accept: a name only |
| kkt-agent-commands takes the admin endpoint's path in `AgentParts`, with no arrow to kkt-mcp | 10.3 (#497) | Accept: one arrow fewer, and kkt-mcp's build script stays out of the assembly's build |

**Phase 12, for Sam: the crates it made.** Each third-party dependency is
now a contract crate in kkt's terms and a wrapper that implements it, and
the program injects the wrapper with no adapter (msg 3041). Wrappers are
named for the tool, or `-os` where the work is the operating system's own
(survey answer 4).

| Contract crate | Wrapper | What the wrapper wraps | Step |
|---|---|---|---|
| kkt-remote-access (was logic) | kkt-remote-access-tailscale (replaces kkt-remote-access-os) | The `tailscale` command | 12.1 (#507) |
| kkt-container (was logic) | kkt-container-docker (replaces kkt-container-os) | `docker` and `docker compose` | 12.2 (#510) |
| kkt-service (was logic) | kkt-service-launchd, kkt-service-systemd | `launchctl` and the plist; `systemctl`, `journalctl` and the unit file | 12.3 (#509) |
| kkt-agent-client (was effect) | kkt-agent-client-rmcp (the old crate, moved) | rmcp and reqwest, reaching kkt's MCP endpoints | 12.4 (#508) |
| kkt-lane-commit (new) | kkt-lane-commit-git | The `git` command | 12.5a (#512) |
| kkt-probe (new) | kkt-probe-reqwest (kkt-diagnostics-os, renamed) | reqwest, for `kkt doctor`'s HTTP probe | 12.5b (#514) |
| kkt-client-command (new) | kkt-client-command-claude | Claude Code's `claude` command | 12.5c (#511) |
| kkt-browser (new) | kkt-browser-os | The system's opener, `open` or `xdg-open` | 12.5d (#513) |

The crates that already implemented a project contract directly before phase 12 are listed under "Already split" in the survey below: kkt-db-sqlite, kkt-clock-system, kkt-entropy-os, kkt-logs-os and kkt-machine-os.

One crate is new without being a wrapper: **kkt-agent-files-os**, the agent
credential and watch-state files (12.4). They are the machine's own, but
they had to leave kkt-agent-client, which became a contract and may hold
no `real`.

**Phase 12, for Sam: where it differs from the survey's table.**

| Difference | Step | Why |
|---|---|---|
| The git split is kkt-lane-commit and kkt-lane-commit-git, not kkt-lane-files-git; the lane files stay behind kkt-lane-files' `Disk`, with the program's adapter | 12.5a (#512, msg 3120) | A crate named for git that also read and wrote the folder's files would be misnamed, and the files are the machine's own, outside the ruling |
| The `claude` split is kkt-client-command and kkt-client-command-claude, not kkt-client-config-claude; the client files stay behind kkt-client-config's `Host`, with the program's adapter | 12.5c (#511, msg 3084) | The tentative name read as an implementation of kkt-client-config, a logic crate; an effect crate may not implement a logic crate's own contract, and the files are the machine's own |
| kkt-agent-files-os is new: the credential and state files leave kkt-agent-client | 12.4 (#508) | A contract may hold no `real`; the files are the machine's own, so they take an `-os` crate |
| kkt-service-os stays, holding only `try_bind`; launchd's waits and its retry wording move into kkt-service-launchd | 12.3 (#509) | Whether an address can be bound is the machine's own; the waits and wording are launchd's, not the contract's |
| kkt-container's contract fields are `sharing`, not `tailscale`; the setup record's stored `tailscale` field is unchanged | 12.2 (#510, msg 3093) | A contract names no tool; the stored record is an existing install's |
| The browser is given to the Docker wrapper at construction (`Docker::new`), not through `OperateParts`; kkt-container's `LoginLink` keeps the terminal's half | 12.5d (#513, msg 3115) | The wrapper opens the link while it waits for the login, so the contract's calls do not change |
| The probe's wrapper is kkt-diagnostics-os renamed, not a new crate | 12.5b (#514) | It already wrapped reqwest; it now implements kkt-probe's `Prober` directly |

**Phase 12, for Sam: the adapters the program still holds, and why.**
After phase 12 none of them stands between kkt and a third-party tool or
library: each reaches the machine's own files, terminal or paths, or the
program's own serving code. They adapt contracts that a logic crate or an
assembly declares, which an effect crate may not implement (the strict
rule), so the program does. kkt-service-os is kept for one thing only:
`try_bind`, whether the service's server could listen on its address,
which the program passes to kkt-operate as a plain function, with no
adapter.

| Adapter (in the program) | Contract it answers | What it reaches | Why it stays |
|---|---|---|---|
| `ServeReads`, `ServiceArgs`, `ServiceCheck` (`machine/operate.rs`) | kkt-operate's `ServeFacts`, `ServiceDefaults`, `CheckService` | The program's own serving settings and config file (`machine/settings.rs`) | The local server's facts are the program's (msg 2876) |
| `ConnectionReads` (`machine/connection.rs`) | kkt-setup's `ConnectionSource` | The setup record and the serving settings | The same: how a client reaches the local server |
| `AdminKeyHere` (`machine/agent_parts.rs`) | kkt-agent-commands' `AdminKey` | kkt-operate's key finder, over the operator's parts | Assemblies may not depend on each other, so the program asks one for the other |
| `NativeDatabases` (`machine/operate.rs`) | kkt-operate's `Databases` | The program's `machine/database.rs`, over kkt-db-sqlite | Choosing and opening the database is serving; kkt-db-sqlite already implements kkt-db's contract directly |
| `NativeSetupFiles`, `NativeKeyFiles`, `NativeConfigText`, `NativeBackups` (`machine/operate.rs`) | kkt-operate's `SetupFiles`, `KeyFiles`, `ConfigText`, `BackupWriter` | kkt-setup-os, kkt-admission-os (with kkt-entropy-os for new keys), kkt-config-file-os, kkt-backup-os | The machine's own files; the contracts are an assembly's |
| `ServerKeyFiles` (`machine/keys.rs`) | kkt-admission's `KeyTexts` | kkt-admission-os' key files | The machine's own files; each program words its key errors itself |
| `KeyFiles`, `ThisProgram` (`machine/agent_parts.rs`) | kkt-agent-commands' `KeyText`, `Program` | kkt-admission-os' key files; kkt-setup-os' path of `kkt` | The machine's own files and paths |
| `CredentialFiles`, `CredentialFile`, `StateFile` (`machine/credentials.rs`) | kkt-agent-commands' `Credentials` and `CredentialSlot`, kkt-watch's `Place` | kkt-agent-files-os | The machine's own files; it applies kkt-setup's credential-file rule |
| `ClientHost` (`machine/client_config.rs`) | kkt-client-config's `Host` | kkt-client-config-os | The machine's own files (msg 3084) |
| `LaneDisk` (`commands/lanes/disk.rs`) | kkt-lane-files' `Disk` | kkt-lane-files-os | The machine's own files (msg 3120) |
| `ContainerConfigs` (`machine/settings.rs`) | kkt-container's `ConfigReader` | The program's own reading of kkt's config file | kkt's own format; toml stays inside kkt-config-file (RUST.md rule 7) |
| `TerminalLines`, `TerminalLink`, `WatchOutput`, `TerminalOutput`, `NativeOutput` | kkt-setup's `Lines`, kkt-container's `LoginLink`, kkt-watch's `Report`, the assemblies' `Output` | kkt-terminal-os | The machine's own terminal; each consumer declares only what it uses (3.1.1) |

## Rules for every step

- **The full checks pass:** `cargo test`, `cargo clippy --all-targets -- -D
  warnings`, `cargo fmt --check`, and `.github/scripts/check-crates.sh`. CI
  runs them on the pull request (step 0.1 makes CI run on pull requests into
  `refactor`).
- **Existing installs keep working** (CLAUDE.md, design principle 3). No step
  intends to change behaviour. The setup record, the Docker folder, service
  files, keys and key files, the database, client configs, CLI commands and
  flags, and the MCP tools stay exactly as they are. Every pull request says
  so and how it checked. A step that finds it cannot avoid an effect on an
  install stops, and the question goes to Sam (see "What needs Sam").
- **What agents see does not change.** `tests/contract/` stays as it is: no
  step runs `UPDATE_CONTRACT=1`. Every text a user or agent reads (refusals,
  command output, log lines) stays the same unless the step says why.
- **A new crate comes with its paperwork:** its entry in the workspace
  `members` and `default-members`, its row in CLAUDE.md's crate table, its
  entry in `docs/architecture/concerns.json` (layer, kind, concern, what it
  must not do; `generate.py` stops without one), and its crate doc saying what
  it owns.
- **A moved item takes its tests with it.** Tests move with the code they
  test; none is deleted to make a step pass.
- **No compatibility re-exports across steps.** A step that moves an item
  updates every user in the same pull request, so no step leaves a re-export
  for a later one to remove.
- **Each pull request reports the scorecard** before and after (next section),
  from `progress.py` run locally: CI does not build the map on pull requests
  into `refactor`, so only this run shows that `generate.py` still works.
- **A step that moves an item a program wires updates `assembly.json`** in the
  same pull request, so its wiring names the item's new crate.
  `generate.py`, and so the scorecard, stops on wiring that names no item.

## Measuring progress

`map-data.js` stays at the map the proposal was made from (27f1d11).
Regenerating it during the refactor would break `propose.py`, which applies
the proposal to that map. The proposal itself is updated as we go (Sam,
2026-11-11): when a step that differs from it on purpose merges, the
difference is charted into `proposed.json` and `proposed-data.js`, so the
page shows it and it is not forgotten. Only kkt-mt-step1 edits the
proposal's files, in a plan pull request after the step merges; builders add
their row to the differences table below in their own pull request and leave
the proposal's files alone. The scorecard measures against the proposal as it
stands, so its totals move when the proposal does; each such plan pull
request says how.

Step 0.2 adds `progress.py`. It runs `generate.py` with a new `--out` option,
which writes the current map built from the code to `target/tmp` instead of
`map-data.js`, and compares it with the proposed map. It prints a
scorecard:

- **Crates:** proposed crates that exist, proposed crates still missing, and
  crates that exist but the proposal removes.
- **Items:** the proposed map's public items found in their proposed crate,
  out of all of them. Items are matched by crate and path. Renamed items use
  their proposed path.
- **Arrows:** the proposed normal arrows present, missing, and extra (arrows
  in the code that the proposal does not have).
- **Kinds and rules:** kinds.py's checks with the proposed kinds, and the
  proposal's own rules (only programs and compositions depend on effect
  crates; no implementation of a contract another crate declares for itself),
  run on the code.

A step is done when every crate, item and arrow it names matches the
proposal, and it adds no new extra arrow or rule break. The scorecard should
only ever move toward the proposal. The last step reaches all items, all
crates and no extra arrows, or lists each remaining difference with its
reason.

## Working in parallel

Several builders can work at once, each in its own clone and on its own
branch off `refactor`. Steps marked **parallel** in the same phase touch
disjoint crate source folders. They still meet in a few shared files, which
are not real conflicts but need a rebase before merging:

- the root `Cargo.toml` (workspace members and dependencies) and `Cargo.lock`
  (regenerate it with `cargo build` after the rebase);
- CLAUDE.md's crate table and `docs/architecture/concerns.json` (different
  rows);
- the programs' wiring in `src/commands/serve.rs`,
  `programs/kkt-keymode/src/main.rs` and `programs/kkt-fake-product/`, where
  several steps each change a few lines.

Steps marked **sequential** depend on an earlier step, or change code that
many others also change, and should not run beside the steps named.

**One rule for the shared constructors.** Several steps change how kkt-mcp's
endpoints, kkt-web's router and kkt-server's `Parts` are built: 1.6, 1.7,
4.2, 5.3b, 5.4b, 5.5, 6.1, 6.2 and 6.3. Only one pull request that changes
any of these constructors is open at a time; the next starts from it once it
has merged. 3.1d also changes them (kkt-server's `AppConfig` and
`Parts::new`, kkt-rules' `Deps`, kkt-web's `Source`), so it is on this list
too.

Sizes are rough lines changed: S under 300, M 300 to 1,500, L over 1,500. An
L step is split where the plan says so; a builder who finds a step bigger
than its size should split it again and say so on its pull request.

## Temporary breaks

Some steps leave a break of the proposal's rules, or an extra arrow, that a
later step removes. They are listed here, so the scorecard's counts can be
checked against the list. A reviewer does not block a step for a break on
this list; one that is not on it blocks.

| Break | Introduced by | Removed by |
|---|---|---|
| The macOS Rust checks are off in CI: `macos-15` is out of `ci.yml`'s list of systems, so platform-specific code is reviewed by hand again | Sam, 2026-11-12: GitHub's macOS runners are short, and #497's job never got one (#501) | Adding `macos-15` back to the list, as its comment says, or a self-hosted macOS runner |

A step that must add a break says so in its pull request and adds a row here
in the same pull request.

With the refactor done, the table guards later changes: `check-programs.py`
runs in CI, and a program that does machine work in its own code again fails
it unless a row here names the program and the file.

## Differences from the proposed map

Some steps end up differing from the proposed map on purpose, for good, not
for a while. They are listed here so Sam can review them in one place. A step
that differs on purpose adds a row in its own pull request; once it merges,
the difference is charted into the proposal (see "Measuring progress"), and
the last column says so.

| Difference | Step | Why | Sam | On the map |
|---|---|---|---|---|
| kkt-lane-files declares a narrow git contract, and the program adapts kkt-lane-files-os' git runner to it, beside `Disk` | 2.6 (#413) | Git's meaning (expected absences, the dirty suffix, error wording) is logic and stays in kkt-lane-files; only running the command is an effect | accepted by Sam, 2026-11-12 | overtaken by 12.5a: git's meaning is kkt-lane-commit-git's, behind kkt-lane-commit's contract |
| One program adapter, an enum over kkt-agent-client's `Remote` and `AdminRemote`, meets kkt-watch's `Server`, in place of two | 1.9 (#420) | Fewer methods (5 in place of 5 + 5), the same arrows | accepted by Sam, 2026-11-12 | overtaken by 12.4: the adapter is gone, and kkt-agent-client-rmcp's `Remote` and `AdminRemote` implement kkt-agent-client's `Watched` directly |
| kkt-container's `tailscale_auth_key` and `docker_env` accept a possibly unsized `Host` | 10.2b (setup) | Setup now passes the plan's injected host trait object to the existing file readers; the widening accepts all previous callers and changes no reads or output | awaiting Sam | no arrow or slot change |
| `ServeFacts::service_settings` returns service-plan inputs and a deferred `CheckService`, which retains opaque program options and their source context | 10.2b (service) | The assembly owns planning and its ordered check/install sequence; the program alone knows and validates its whole serving surface. No serving option type or assembly dependency crosses the boundary | awaiting Sam | yes (10.4) |
| A third-party tool has a project-scope contract plus a named wrapper implementing it directly, regardless of consumer count; the program only injects it. The effect wrapper owns tool knowledge and a private runner, with running behind `real` | 12.1 | The scope is the abstraction consumers use; keeping one tool's knowledge and execution together avoids a shared process concern. The remote-access pilot replaces `Host::run` and the program adapter with `RemoteAccess` and `Tailscale` | decided by Sam, 2026-11-12 (msgs 3041, 3050) | yes |
| `ServiceDefaults::matches_defaults(args)` answers the installed/default settings comparison; only service-capable `OperateParts` can ask it | 10.2a | Both serving surfaces stay in the program. A service-capable adapter requires `ServeArgs` at construction; record/config-only parts have no such capability. Installed arguments are resolved before defaults; the assembly retains the service-file check and refusal wrapping | required by the 10.2a review; awaiting Sam for the narrowed question | yes (10.4) |
| `ServerSettings::check` returns opaque `CheckedSettings` containing the immutable settings and their host policy; `Parts::new` consumes that one value | 9.2 | The constructor cannot skip validation or combine settings with a different host policy; the program reads the checked values for its record. The integration harness uses the program's own run-record mapping | requested in the 9.2 review; awaiting Sam for the constructor detail | yes (10.4) |
| kkt's `new_server_run` is in `machine::server_run`, public, not in `commands::serve` | 9.2, 9.3 | The integration tests' harness records a run as `kkt serve` does, with the program's own mapping, in place of a third copy of it (the 9.2 review) | on the list of new differences for Sam (in "Awaiting Sam") | yes (10.4) |
| kkt-operate's module for `kkt agent` is `agent`, not `agents` | 10.2a (#496) | It holds one command, `kkt agent`, as `lanes` and `key` hold theirs | on the list of new differences for Sam (in "Awaiting Sam") | yes (10.4) |
| kkt-agent-commands takes the admin endpoint's path in `AgentParts` (`admin_path`), and has no arrow to kkt-mcp | 10.3 (#497) | The arrow was for one constant, `MCP_ADMIN_PATH`, and brought kkt-mcp's build script into the assembly's build; kkt-setup's `admin_endpoint` already takes the path from its caller | on the list of new differences for Sam (in "Awaiting Sam") | yes (10.3) |
| The agent credential and watch-state files leave kkt-agent-client for a crate of their own, kkt-agent-files-os, and the program keeps adapting them to kkt-agent-commands' `Credentials` | 12.4 | kkt-agent-client becomes a contract, which may hold no `real`; the files are the machine's, not a third-party tool's, so they take an `-os` crate (Sam's answer 4), and kkt-setup's credential-file rule stays the program's to apply | to show Sam with phase 12 | yes (12.4) |
| kkt-service-os reports with its own `Finished`, which the program translates to kkt-service's in `run` and `run_attached` | 2.2 (#427) | So kkt-service-os depends on neither kkt-service nor kkt-config; two adapter methods translate | accepted by Sam, 2026-11-12 | overtaken by 12.3: kkt-service-launchd and kkt-service-systemd implement kkt-service's `Service` directly; kkt-service-os keeps only `try_bind` |
| kkt-remote-access-os returns its own output and I/O error, which the program maps to the remote-access host contract | 2.3 (#436) | The effect has no dependencies; the adapter preserves all output fields and launch-failure advice | accepted by Sam, 2026-11-12 | overtaken by 12.1: kkt-remote-access-tailscale implements `RemoteAccess` directly, and the adapter is gone |
| kkt-client-config-os states its own 0600 private-file mode beside the file operations | 2.4 (#437) | The same fixed value moves with its writer early, so the effect depends on neither client-config nor config | accepted by Sam, 2026-11-12 | not a map change |
| kkt-container-os' command runner takes and returns values of its own, which the program translates to kkt-container's `Command` and `Finished`, and it states its own private file mode | 2.1 (#422) | So kkt-container-os depends on neither kkt-container nor kkt-config; two adapter methods translate instead of forwarding | accepted by Sam, 2026-11-12 | overtaken by 12.2: kkt-container-docker implements `Container` directly, and the adapter is gone |
| kkt, kkt-keymode and kkt-fake-product depend on kkt-format (later kkt-mcp-formatter), kkt-web, kkt-events and kkt-mcp, arrows the proposal does not draw | 3.1d (#424), 3.1a (#429), 3.1b (#431) | The program fills each crate's own settings, so it names their types; the proposal draws those types only from kkt-server | accepted by Sam, 2026-11-11 | yes |
| Each crate takes its tuning values as its own `Settings` (kkt-db-sqlite's `Options`) with a `DEFAULT`, which the program passes, where the proposal shows item 7's constants; a constant only other crates' tests read sits in its crate's `testing` module; the fake product fills each part's settings itself and does not depend on kkt-settings | 3.1a (#429), 3.1b (#431), 3.1c-1 (#445) | Item 7 marked these values "injected"; a settings struct with its default is how a crate takes them, and the program can still pass the default unchanged | accepted by Sam, 2026-11-12 | yes |
| kkt-diagnostics has public `DOCTOR_MAX_RESPONSE_BYTES` and `DOCTOR_PROTOCOL_VERSION`, which the program passes to kkt-diagnostics-os' `Settings`; kkt-agent-client's `Settings` is outside `real` | 3.1c-1 follow-up (#445) | kkt-diagnostics-os chooses no values of its own, and depends on no other crate (2.5); the client's settings are plain data, which the program fills without `real` | accepted by Sam, 2026-11-12 | yes |
| The 0.10.0 migration test stays in the program's tests; only `legacy()`'s facts move into kkt-rules | 5.2b-1 | It builds real SQLite files, and F16 keeps effect crates out of kkt-rules' tests | accepted by Sam, 2026-11-11 | not a map change |
| kkt-diagnostics-os' probe still interprets the server's answers (status meanings, advice text) as it sends them | 2.5 | Splitting it needs the probe's sequence to move into kkt-diagnostics; a candidate follow-up | accepted by Sam, 2026-11-11 | yes; since 12.5b the probe is kkt-probe-reqwest, which implements kkt-probe's `Prober` directly and still reads the answers |
| Backup logic supplies an immutable Layout (names, copies, metadata, modes); backup-os writes it and asks logic which old names to remove | 2.7 (#440) | Keeps backup decisions in logic while the effect depends only on backup, as the proposal draws | accepted by Sam, 2026-11-12 | yes |
| A package's own `testing` feature may turn on a workspace dependency's `testing` (check-crates rule 3) | 5.2b-1 | kkt-rules' test helpers need kkt-db's in-memory database; `testing` is still reached only from dev-dependencies | decided by Sam, 2026-11-11 | not a map change |
| kkt-config-file-os has five readers and writers, not three: `read` (setup's edit: a missing file is empty), `read_default` (`kkt serve`'s default file: skipped if it cannot be seen), `read_named` (a `--config` file: must be there), `read_instructions` and `write`, with its own `ReadError` and `WriteError` | 3.3 (#445) | Each keeps the behaviour and messages its caller had; the error types are the crate's own, not std's | accepted by Sam, 2026-11-12 | yes |
| kkt-config-file's `parse_file` takes the caller's log-level check, and `Config` carries what it makes of the levels; kkt-container's `ConfigReader` returns its own `ContainerConfig` | 3.3 (#445) | kkt-config-file need not know kkt-logs, and of two bad settings the first by name is still the one named, as before | accepted by Sam, 2026-11-12 | yes |
| `config_home` is a method of kkt-machine's `Env`, with `absolute_xdg_config_home` and `absolute_xdg_data_home`, not a function of kkt-config-file; `config_dir` and `config_file` take the optional config-home folder, with no kkt-config-file dependency on kkt-machine; `kkt_admission::access_key_file` and kkt-setup's record paths take the config folder, not `Env`; the program has `NewKey` and `NewAvatar` | 3.4 (#448) | `config_home` is shared by other programs' config (kkt-service's and kkt-client-config's), and the XDG values count only when absolute, so `Env` gives them in one place; a path inside kkt's config folder takes that folder; `NewKey` and `NewAvatar` group two commands' arguments to stay within clippy's limit | decided by Sam, 2026-11-12: kkt-config-file takes the folder, not Env; other placements accepted by Sam, 2026-11-12 | yes: the arrow is gone (10.4) |
| `kkt_service::Manager::for_this_os` takes the optional home and config-home paths, not `Env`; kkt-service no longer depends on kkt-machine. kkt-setup, kkt-client-config and kkt-container keep the environment record | 3.4 follow-up (audit msg 2788) | The service chooses one path per supported target; each of the other three reads several facts. The caller passes `env.home` and `env.config_home()`; platform choices, missing-home and unsupported-target behaviour stay the same | decided by Sam, 2026-11-12 | yes |
| `serve_bind` stays in the program; the service's bind check is kkt-service's `check_bind`, supplied with kkt-service-os's `try_bind` by the program; kkt-keymode keeps four key-refusal messages of its own beside kkt-admission's `key_refusal` | 3.5b (#449) | `serve_bind` gathers the settings from the machine, which kkt-hosts cannot; `check_bind` decides and words the refusal, and trying the address is an effect of service-os (7.3a); kkt-keymode's messages differ from kkt's, and its output stays the same | to re-show Sam after 9.3 | yes |
| kkt-settings has no `SettingsSource`: kkt and kkt-keymode turn kkt-config-file's `Config` and the flags each into kkt-settings' `ServeOptions`, which kkt-settings combines; `Sources`, `Loaded` and `read_config` stay in the program | 3.3 (#445), 3.5c (#450) | `ServeOptions` already is a plain record of one source's optional settings; a trait over it would add a method and two adapters with no new behaviour | to re-show Sam after 9.3 | yes |
| kkt-settings depends on kkt-admission (`AccessKey` in `ServeSettings`, the key paths), kkt-logs (`LogLevel`, `LogPolicy`), kkt-mcp-formatter and kkt-web (the bounds `Limits::check` reads, now public) and kkt-names (`NamePack`) | 3.1b (#431), 3.5b (#449), 3.5c (#450) | Its values hold those types, and `Limits::check` reads the bounds their owners state. None of them depends on kkt-settings, and only the programs do | not accepted by Sam, 2026-11-12: kkt-settings goes (phase 9) | yes; phase 9's chart removes the crate and these arrows |
| Each program keeps `check_settings`: a short function that turns kkt-settings' values into each crate's settings and calls each owner's check, with the mapping of `Limits` to kkt-rules' and kkt-web's settings (about 25 lines) in both kkt and kkt-keymode; `Limits::check` stays one ordered list; kkt-keymode keeps its own `KeyFiles` | 3.5c (#450) | No allowed owner exists for the mapping: kkt-rules and kkt-web may not know kkt-settings, nor kkt-settings them. One ordered list keeps the first bad limit named the same. `KeyFiles` goes with the proposal's kkt-admission-os (7.2) | superseded by phase 9: kkt-server keeps the order, and each program maps its keys onto `ServerSettings`; to re-show Sam after 9.2 and 9.3 | yes; phase 9 charted |
| kkt-settings words the settings a run records (`RecordedSettings`); kkt-provenance's `RunSettings` holds those lines and depends only on kkt-hash; kkt-service's plan takes its own `ServiceSettings`; kkt-container's `status` and `lane_limit` take their defaults from the program; `DEFAULT_ALLOW_ANY_BIND` is private | 3.5c (#450) | The wording moved unchanged with the golden tests that pin it, and the run record stays byte for byte the same; kkt-settings' defaults are `Limits::DEFAULT`'s, so no separate `DEFAULT_NAME_PACK` | superseded by phase 9 for the wording, which moves to kkt-server, to re-show Sam after 9.2; the rest accepted by Sam, 2026-11-12 | yes; phase 9 charted |
| kkt-logs supplies explicit pure rollover and age-pruning plans, which logs-os performs | 4.1 | Keeps thresholds, rename order and expiry rules in logic while retaining the writer's existing I/O order | accepted by Sam, 2026-11-12 | yes |
| kkt-logs' plans are `should_roll`, `rotation` (with `Rotation`), `prune_due` and `pruning` (with `Pruning` and its `expired`), in place of `rolls_over`, `rolled_name` and `expired`; `LOG_PRUNE_INTERVAL` stays a private constant, not a setting; kkt-logs has its own `SettingError`; kkt-logs-os keeps the program's names `init_logging` and `init_stderr_logging`, not `start` and `start_stderr` | 3.1c-2, 4.1 (#452) | Found when charting 4.1, with no row of their own. A plan names a whole rollover or pruning, so the writer only carries it out; the interval kept the fixed value it had; kkt-logs checks its own settings and may not use kkt-settings' error, as kkt-web, kkt-admission and kkt-hosts; the start functions moved unchanged | accepted by Sam, 2026-11-12 | yes |
| The shared log contract carries borrowed named Field/Value records with Display or Debug formatting, Level::Trace, and testing::Entry | 4.2 | Preserves all five severity levels and structured fields without exposing tracing types or flattening fields into text; Entry retains captured events for assertions | accepted by Sam, 2026-11-12 | yes |
| kkt-api-agent's `Audit::AdminEndpoint` holds the admin key's name as text (`Option<String>`), not kkt-names' `AdminName` | 5.3a | A contract crate holds the already-checked name as a plain value and depends on no kkt crate; kkt-names checks the name where kkt-identity-keymode builds the `Audit` (msg 2476) | accepted by Sam, 2026-11-12 | not a map change |
| kkt-api-agent also holds `LaneSource`, which the proposal left in kkt-rules | 5.3a (#451) | `Lane`, a shared record, holds it, and a contract crate may not hold a record of kkt-rules; found when charting, with no row of its own | accepted by Sam, 2026-11-12 | yes |
| kkt-api-operator also holds `Overview`, `ChannelView` and `LaneHistory`, with `Overview`'s `AgentActivity` and `ActivityCount`, which the proposal's list of its records left out | 5.4a | `Views::overview`, `Views::channel_view` and `Lanes::lane_history` return them, and a contract must own what it returns (msg 2503) | accepted by Sam, 2026-11-12 | yes |
| kkt-api-operator's operations return an `Answer`, a boxed future of their result; since 5.3b the type is kkt-api-agent's, shared by all three kkt-api crates | 5.4a (#455), 5.3b | So the traits work through `dyn`, as kkt-identity's `Identify` does; found when charting, with no row of its own | accepted by Sam, 2026-11-12 | yes |
| Terminal supplies flush_output, message and output_is_terminal beside output and errors | 7.1a | Preserves prompt flushing, each watch message's locked write and flush, and the browser's separate stdout-terminal check | accepted by Sam, 2026-11-12 | yes |
| Terminal::interactive checks stdin, while output_is_terminal checks stdout separately | 7.1b | Preserves setup asking when stdin is a terminal even if stdout is redirected; browser opening still requires both | accepted by Sam, 2026-11-12 | yes |
| Key-file helpers take a caller-supplied key factory; make_default_key reports its created path, write_admin_name takes bound text, and KeyFiles adds read, read_optional and create_dir_all with ReadError retaining its path and I/O cause | 7.2 | Keeps admission-os dependent only on admission; the program preserves its info-event target and admin-name wording, and the two KeyTexts adapters translate native read errors into each program's existing refusal messages | accepted by Sam, 2026-11-12 | yes |
| kkt-rules' tests use kkt-db-sqlite's in-memory database, an effect crate's, not kkt-db's `MemoryPlace` | 5.3b; undone in 11.2 | Speed: the 200 tests took under 5 seconds on SQLite in memory and over 10 minutes on kkt-db's `MemoryPlace`, which copies the whole database on every change. Since 11.2 they run on kkt-db's `FastPlace` (11.1) in about 6 seconds, and kkt-rules no longer depends on kkt-db-sqlite at all | not accepted by Sam, 2026-11-12: a fast stand-in instead (msg 2815) | yes: the arrow is gone, and kkt-db's `testing::FastPlace` is charted (10.4) |
| Two combined traits: kkt-api-operator's `OperatorApi` (`Api` + `Views` + `Changes` + `Lanes` + `Server`, with `in_swarm_operator`) and kkt-api-admin's `AdminApi` (`OperatorApi` + `Admin`, with `in_swarm_admin`); kkt-api-admin depends on kkt-api-operator | 5.4b | An endpoint that moves into the caller's swarm needs every operation it uses in that swarm, which `Api::in_swarm` alone cannot give; `Rules` implements both, so no adapter (msg 2562) | accepted by Sam, 2026-11-12 | yes |
| kkt-mcp-formatter no longer depends on kkt-rules (only its tests do, for kkt-rules' default settings) | 5.5 | The proposal kept the arrow from the old map; with the wording in kkt-rules and the records in the kkt-api crates, the formatter's library uses nothing of kkt-rules (msg 2580) | accepted by Sam, 2026-11-12 | yes |
| kkt-web's `Sessions<T>` is generic over what a session holds (bound by its `Held`), `Gate::SignedIn` (kkt-server's `Principal`, the preview's `()`); `Gate::admits` takes kkt-web's `Session<T>` (absent, ended or live) and `Gate::sign_in` returns `SignIn<T>` (admitted, or refused with a reason) in place of a session ID; `Sessions::new` takes an `Access` (open or sign-in), from which the page knows whether sign-in is required; the lifetime is `session_lifetime` in kkt-web's `Settings`, so kkt-server's `AppConfig` loses `sessions` and the program's `session_settings` goes (its `web_settings` fills the lifetime); kkt-web's `testing::Door` holds a gate and its sessions for the page scenarios | 6.1 | The page keeps the sessions, so the gate is asked about what a session holds, not its ID; form tokens need to know whether sign-in is required, so the sessions know it from construction and `Gate` drops `sign_in_required`; one settings value per crate | accepted by Sam, 2026-11-12 | yes |
| kkt-admission's `WebAdmission` decides over its own `WebSession` (absent, ended or live with a `Principal`), and `WebAdmission::sign_in` returns a `Decision`; its `SignIn` and `Settings` go | 6.1 | kkt-admission keeps the access decision and its refusal texts ("sign in first", "the session has ended; sign in again") without knowing kkt-web's sessions | accepted by Sam, 2026-11-12 | yes |
| kkt-api-operator's `Views` has `subscribe`, which gives the operator's `Notices` (an async `next` of a `Notice`, or `None` at the end), with its own notice records (`NewAgent`, `ChannelCreated`, `Joined`, `Left`, `ChannelHidden`, `AgentHidden`, `ChannelMembers`, and `Notice::Missed` with no count; a message is kkt-api-agent's `Message`); it checks `Read`. `Rules` adapts kkt-events' subscription to it, and kkt-web serializes the records for its live feed; kkt-web's `LiveEvent`, `live_event`, `NewAgent`, `NewChannel`, `Joined`, `Left` and `testing::ChannelHidden`, which the proposal keeps, go | 6.2 | The proposal has kkt-web draw live updates from kkt-api-operator's subscribe but names no types for it, and a contract may not name kkt-events; the records carry exactly the fields the page sends, in the same order, so the feed is unchanged | accepted by Sam, 2026-11-12 | yes |
| kkt-web's `Gate` has no `Viewer` type of its own choosing: it admits a `Viewer` record of kkt-web (a `SwarmId` and an `Operator`), and kkt-server's `WebViewer` goes; the page takes the application as one `Arc<dyn OperatorApi>` and asks `in_swarm_operator` for each request's viewer | 6.2 | The page must call the operations in the viewer's swarm as the viewer's operator, so it must read both; the proposal left `Viewer` opaque while removing `Sources`, which read it | accepted by Sam, 2026-11-12 | yes |
| kkt-web's `Settings` holds `page: PageLimits` in place of `page_messages` and `member_excerpt_chars`; the programs fill it, and kkt-server passes it in `Settings` as before | 6.2 | One settings value per crate; `PageLimits` is the part only the page applies | accepted by Sam, 2026-11-12 | yes |
| The preview keeps its simulated world in records of its own (kkt-api-sim's private `records`, the old contract's records) and translates them to the kkt-api crates' records in its `api` module; it refuses every operation the page never asks; it makes a world for each swarm asked for, the first time | 6.2, kept in 6.3 | Changes the preview's internals as little as possible; a world per swarm lets kkt-web's own test see that each request reads its viewer's swarm | accepted by Sam, 2026-11-12 | yes |
| kkt-web's `testing` scenarios take an `App` (an `OperatorApi` and the operator the scenario checks as) in place of a `Source` | 6.2 | The scenarios read the application to check what the page did, and every operator operation takes the operator | accepted by Sam, 2026-11-12 | yes |
| kkt-rules' `start` takes `NewServerRun` and a `Startup` (the rules' `Deps` without the run, which `start` fills in) and returns the `Rules`; `Rules::run_id` gives the run's ID, for the program's log line; kkt-server's `new_server_run` turns kkt-provenance's `ServerRun` into a `NewServerRun`; kkt-server's `Parts` holds the `Rules`, the clock and entropy the page's sessions use, `identify`, `log` and the formatter, and `Parts::new` takes those, in place of the database, wording, events settings and run ID; `AppConfig` loses `rules` (the settings go into `Startup`); the program makes the events bus and stops it with the server's stop; kkt-server's `Deps` wiring leaves assembly.json, and each program's wiring goes to `Startup` and `Parts::new (rules)` | 5.6a | The proposal names `start` and `NewServerRun` but not how `start` gets the rest of `Deps`; one `Startup` struct keeps `Deps` as it is for tests that build `Rules` directly; kkt-rules may not know kkt-provenance, and kkt-server already does, so it maps the run; the program now builds the rules, so it owns the bus the rules hear on, and kkt-server depends on kkt-events only in its tests; kkt-keymode depends on kkt-api-agent for the swarm it names in `Startup` (`SwarmId::main`) | to re-show Sam after 9.2 | yes |
| kkt-server's `Parts` holds the concrete `Rules`, so kkt-server does not name kkt-api-admin's traits; the proposal's arrow kkt-server → kkt-api-admin is missing | 5.6a (#476) | `Parts` passes the one `Rules` to each surface, which takes it as its own contract (`Api`, `OperatorApi`, `AdminApi`); naming the traits in `Parts` would add a type parameter or a boxed value without changing what any surface can reach | decided by Sam, 2026-11-12: `Parts` takes the contract, an `Arc<dyn AdminApi>` (9.2) | yes: 9.2 made the arrow kkt-server → kkt-api-admin (10.4) |
| The store goes private except through kkt-rules' `testing`: the program's tests that pin what a 0.10.0 database answers (`tests/migrations/v0_10_0.rs`, every `Reader` question) and that write rows into a database file (`tests/database_files.rs`) reach `Reader`, `Writer`, the records and filters they take, and `testing::read` and `testing::write`; `testing::database` gives a `Db` for a program's own `start` (the fake product's tests) and the operator's `CommandLine`; `testing::memory` takes a `testing::Memory` (`Startup` without the database, with defaults from `Memory::new`); `Reader::recent_messages`, which no rule asks any more, is behind `testing` | 5.6b (#477); narrowed in 11.4 | The 0.10.0 fixture must answer every pinned read (CLAUDE.md), and those reads are the store's questions, which the kkt-api operations do not ask one by one; the file tests write rows to check how a file is kept, not what a rule does. Behind `testing`, a released build reaches none of it | decided by Sam, 2026-11-12: narrowed to purpose-named functions (11.4): `every_store_read` with `Probes`, `rules_on`, `swarms_stay_apart_in`, `memory`, `database`, `fail_next_write` | yes |
| Credential readers take a CredentialCheck over named CredentialFileFacts and a warning callback; CredentialFile takes a cleanup-warning callback; PrivateMode and CredentialFileFix carry the checked mode, advice and warning, and FileLock exposes LockRefused | 7.4 | Keeps agent-client independent of setup and terminal effects; setup owns the rule over caller-quoted paths, both owner checks and warning-before-read ordering stay intact, and cleanup warnings keep their original log target | accepted by Sam, 2026-11-12 | yes |
| kkt-web-preview is a program only, a binary with no library (kind composition, not logic): its arrows are [dependencies] of the binary, not dev arrows of an example, and its `SettingError` is the binary's own, not a public item; the preview's tests (the page scenarios over `Preview`, and tests of its world, feed and gate) are kkt-api-sim's unit tests, beside the stand-in they test; the program's own tests are of its flags | 6.3 | Decision 39 leaves kkt-web-preview only the program (A7: it counts as a program), and check-crates.sh lets a dev-only program, not an example, turn on `testing`; tests of a stand-in sit beside it | accepted by Sam, 2026-11-12, with its reliance on rule 3's dev-only `testing` exception | yes |
| kkt-server has `page`, which builds an `App` of the web page alone over any `OperatorApi` and gate; kkt-web-preview serves it through kkt-server-listen, so it depends on kkt-server and kkt-server-listen, arrows the proposal does not draw | 6.3 | So the preview program serves the page without naming axum, which stays in kkt-web, kkt-mcp, kkt-server and kkt-server-listen, and without binding a socket in its own code | accepted by Sam, 2026-11-12 | yes |
| kkt-web-preview logs through kkt-logs-os (`init_stderr_logging`, `TracingLog`, with kkt-logs' `LogLevel` and kkt-log's `Level`) and reads its key file through kkt-admission-os (`KeyFiles::read`), arrows the proposal does not draw; it drops `tracing` and `tracing-subscriber` | 6.3 | So the program does no machine work in its own code (check-programs.py) | accepted by Sam, 2026-11-12 | yes |
| kkt-api-sim has `PreviewSettings` (the feed's steps per minute) and its own `SettingError`; the page size is kkt-web-preview's flag, checked by the program (its `SettingError`) into kkt-web's `PageLimits`; the preview's bind address, seed and session lifetime are the program's (the lifetime is kkt-web's default); kkt-web's `SettingError`, which only the preview used, goes; kkt-api-sim's `PREVIEW_SWARM`, `preview_viewer` and `Preview::swarms` are public, for kkt-web's viewing test | 6.3 | Each crate declares the settings it takes and its check's error (decision 42); the simulation never used the page size | accepted by Sam, 2026-11-12 | yes |
| kkt-api-sim depends on kkt-shutdown: `Preview::new` takes the stop that ends its live notices and `Preview::run` the one that ends its feed; kkt-web no longer depends on kkt-events, which only the preview used since 6.2 | 6.3 | The stop is the caller's, as it was in kkt-web; the page hears notices through kkt-api-operator's `Notices` (6.2) | accepted by Sam, 2026-11-12 | yes |
| kkt-mcp's tests do not use kkt-api-sim | 6.3 | They already use only stand-ins (the fixed clock, counting entropy, the rules over an in-memory database) and need nothing the simulation gives; the arrow can come when a test does | accepted by Sam, 2026-11-12 | yes |
| kkt-fake-product's tests turn on kkt-agent-client's `real` (a dev-dependency): a documented exception to "tests SHOULD NOT turn on `real`", listed in `.github/scripts/real-exceptions.json` | 7.5; fixed after 8.1 | Only its integration tests use this dependency (msg 2604): they drive a running server over HTTP with kkt-agent-client's client, which only `real` builds. Until Sam's decision it was a normal dependency, since check-crates allowed `real` in dev-dependencies only for examples | decided by Sam, 2026-11-12: SHOULD, with this documented exception | yes: a test arrow, with `real` |
| Http returns an opaque HttpReply with status, header and text/byte-stream methods; SqlitePlace::create reports CreateError::Exists or Open | 7.5 | Keeps HTTP library types inside agent-client and preserves streaming/form behavior; the program keeps its existing-file and failed-open wording | accepted by Sam, 2026-11-12 | yes |
| Setup's client/admin connection and endpoint decisions use consumer-owned `ConnectionSource` and `ServeConnection<S>` records; recorded selection takes a path-resolution callback, and `recorded_setup` takes lazy unfinished/unrecorded readers | 7.3a | Preserves the first read/error and avoids setup depending on program settings, diagnostics or native providers | accepted by Sam, 2026-11-12 | yes |
| Setup-os adds `path_dirs`, `absolute`, `canonicalize`, `write_unfinished` and `remove_unfinished`; container Host and native key files expose `is_file` | 7.3a | PATH is captured once for all client checks; path I/O stays native; note writes keep their original rename and missing-file semantics; install presence distinguishes files from directories | accepted by Sam, 2026-11-12 | yes |
| `docker_env` returns settings text or the original read refusal, rather than an absent value for a missing file | 7.3a | Upgrade previously refused a missing .env; Host now performs that same read and the caller keeps its version-line refusal | accepted by Sam, 2026-11-12 | yes (`docker_env`'s description) |
| Setup returns plain `DoctorCheck` and lane-import `Outcome` values, adapted by the program from/to the diagnostics and operator records; service checks receive installed/running facts | 7.3b | Owns each check, fix and import report without setup depending on diagnostics, service or operator implementation crates; program adapters copy facts only | accepted by Sam, 2026-11-12 | yes |
| Setup adds status-formatting, backup-folder, uninstall-record, key-naming and lane-limit functions; setup-os removes records, SqlitePlace checks file presence and KeyFiles::try_exists retains probe errors | 7.3b | Moves the remaining program-work decisions and file probes to their owners while preserving read order, messages and missing-file semantics, including path calls the original scanner missed | accepted by Sam, 2026-11-12 | yes: the proposal's `server_key_file_name` is `client_key`, with `staged_client_key` |
| kkt's `local_web_key` and `local_operator_key` stay in the program, not kkt-setup: they read the record, the config folder and the configured key in kkt-setup's order, and kkt-setup's `operator_key` and `configured_key` decide | 7.3b (#473) | Found when charting, with no row of its own: the decision is already kkt-setup's, and what is left is the reads | to re-show Sam after 10.2a | yes |
| kkt-server's tests use kkt-api-sim's `Preview`, `PreviewGate` and `PreviewSettings`, and kkt-web's tests stop the simulated feed with kkt-shutdown's `Shutdown` (test arrows the proposal does not draw) | 6.3 (#470) | Found when charting, with no row of its own: the page tests over the simulated application moved with it, and a manual stop is one of the stand-ins the proposal gives kkt-web's tests | accepted by Sam, 2026-11-12 | yes |
| Admission's `server_keys::bound_to` and `server_keys::bound_in` are free functions over names' `AdminName`, and names drops its hash dependency; `server_keys::admin_name_line` takes the Docker-copy fact and a lazy name-text reader, and `server_keys::admin_name_written` formats the native write result | 7.7 (#475) | Binding belongs to admission while name validation stays in names; cross-crate inherent methods are not possible, and the callback preserves the host-copy no-read rule without an admission-to-container/native dependency | accepted by Sam, 2026-11-12 | yes |


## The steps

### Phase 0: set up

**0.1 CI runs on pull requests into `refactor`.** S. CI's workflow names only
`next` and `main` (`.github/workflows/ci.yml`, lines 5 and 7), so today a pull
request into `refactor` runs nothing. Add `refactor` to both lists. Nothing
else in CI names a branch except the crate-map job, which runs only into
`main`. Depends on nothing. Done when CI runs on a pull request into
`refactor`. This change is in the plan's own pull request, so the plan's pull
request is the first to run CI.

**0.2 The scorecard.** S. `generate.py --out <file>`, and `progress.py` as
described above, with a line in `docs/architecture/README.md`. It changes no
code. Depends on nothing. Done when `progress.py` prints the baseline, which
the pull request records.

### Phase 1: foundations (two lanes)

These are small, low-risk and used widely, so they go first. They are not in
disjoint folders: several change the same users. So they run in two lanes,
one step at a time in each, and the two lanes run beside each other:

- **Lane A**, which changes kkt-mcp, kkt-server, kkt-identity, kkt-names,
  kkt-admission and kkt-rules: 1.3, then 1.1, then 1.6, then 1.7.
- **Lane B**, which changes the programs and `tests/common`: 1.2, then 1.4,
  then 1.5, then 1.9.

1.8 changes only kkt-db, kkt-db-sqlite and kkt-store, and can run beside
both lanes.

**1.1 kkt-hash.** S. New logic crate with `sha256`. kkt-admission, kkt-names,
kkt-provenance, kkt-rules and the program (`src/commands/setup/steps.rs`) call
it instead of sha2, which leaves their manifests. Depends on nothing beyond its lane.
Done when sha2 is only in kkt-hash.

**1.2 kkt-clock-system.** M. `SystemClock` and kkt-clock's `real` feature
move to the new effect crate. kkt-clock becomes a contract crate. Users: the
three programs, `src/logging.rs`, kkt-web's tests and `tests/common`. Depends
on nothing. Done when kkt-clock has no `real` feature and the scorecard's
kkt-clock and kkt-clock-system items match.

**1.3 kkt-entropy and kkt-entropy-os.** M. `Entropy` (with `random_chars`
as a provided method), `SeqEntropy` and `SeededEntropy` (behind `testing`)
move from kkt-names to the new contract crate kkt-entropy. `OsEntropy` moves
to kkt-entropy-os. `ALPHABET` becomes private to kkt-rules. kkt-names keeps
the names, as logic. Users: kkt-rules, kkt-admission, kkt-web, kkt-server,
kkt-mcp, kkt-identity, the programs and `tests/common`. Depends on nothing beyond its lane.
Done when kkt-names has no `real` feature and the scorecard's kkt-entropy
items match.

**1.4 kkt-shutdown-os.** S. `StopSignals` and kkt-shutdown's `real` feature
move to the new effect crate. `Shutdown` stays a concrete type (F10). Users:
the programs. Depends on nothing beyond its lane.

**1.5 kkt-server-listen.** S. `listen` and `Listening` move out of
kkt-server's `real` feature into the new effect crate, which may use axum
(decisions.md). Users: the programs and `tests/common`. Depends on nothing beyond its lane.

**1.6 kkt-format becomes kkt-mcp-formatter.** M. A rename of the package and
its users (kkt-mcp, kkt-server, the programs). `NoticeWording` stays for now
and moves in step 5.5. Depends on nothing beyond its lane.

**1.7 kkt-identity-keymode.** S. `KeyMode` and its tests move to the new
logic crate. kkt-identity keeps `Identify`, `Identified` and `Identifying` as
a contract crate, and gains `testing::FixedIdentify` behind a new `testing`
feature (F11). Users: the programs and kkt-mcp's and kkt-server's tests.
Depends on nothing beyond its lane. `Identify::avatar` still takes `&Rules`,
so kkt-identity keeps its arrow to kkt-rules until 5.3b changes that
signature: the scorecard shows that arrow as extra until then, and this step
is done without it.

**1.8 kkt-search.** M. `words`, `terms`, `excerpt` and `matches` move from
kkt-db to the new logic crate. `Search`, `Hit` and `Excerpt` stay in kkt-db,
and `excerpt` takes its marks and window as arguments. Touches kkt-db,
kkt-db-sqlite and kkt-store. Depends on nothing. Must land before 5.1.

**1.9 kkt-agent-client stops implementing kkt-watch's Server.** S.
kkt-agent-client gets plain methods with its own types; the program's thin
adapter implements `Server` and maps one `Failure` to the other. Touches
kkt-agent-client, kkt-watch's users and `src/commands/watch.rs`, which 1.2
also changes, so it comes last in lane B.

### Phase 2: operating-system implementations (parallel)

Each step moves one crate's OS implementation into its own `-os` crate, as a
plain type that implements nothing. Its `real` feature goes with it. The
parent crate becomes logic and keeps its contract, and the program adapts the
plain type to that contract. Each is S to M and depends on nothing in phase 1. Each changes its own
crates and the program's uses of its OS type, which are spread over `src/`,
and several share files there (`src/machine/record.rs`,
`src/commands/lifecycle.rs`, `src/commands/setup/steps.rs`). Two can be open
at a time; expect small rebases in `src/`.

- **2.1** kkt-container-os (`OsHost`, adapted to kkt-container's `Host`).
- **2.2** kkt-service-os (`OsSystem`, adapted to kkt-service's `System`).
  `DryRun` stays in kkt-service, implementing its own trait (F13).
- **2.3** kkt-remote-access-os (`OsHost`, adapted to kkt-remote-access's
  `Host`).
- **2.4** kkt-client-config-os (`OsHost`, adapted to kkt-client-config's
  `Host`).
- **2.5** kkt-diagnostics-os (`OsProber`, adapted to kkt-diagnostics'
  `Prober`). It keeps reqwest for its probes (msg 2170).
- **2.6** kkt-lane-files-os (`OsDisk`, adapted to kkt-lane-files' `Disk`).
- **2.7** kkt-backup-os (`write` and `prune`). kkt-backup keeps the names,
  contents and pruning rules.

Done for each when the parent crate has no `real` feature, and the
scorecard's items and arrows for the pair match.

### Phase 3: configuration and settings (sequential)

**3.1 kkt-config's items go to their owners.** The 164 rows of the page's
kkt-config table (`config-split.json`) say where each item goes and how:
constant, setting, injected or check. All of them come out of one file,
`crates/kkt-config/src/lib.rs`, so the four parts run in order:

- **3.1d first:** `Limits` split into each crate's own settings struct, which
  the program fills; `MAX_LANE_DESCRIPTION_CHARS` becomes one of kkt-rules'
  settings (F14). It goes first because it changes what the other parts'
  crates read. M.
- **3.1a** storage, rules and events (kkt-rules, kkt-db-sqlite, kkt-events,
  kkt-names): M.
- **3.1b** the surfaces: the web page, the MCP endpoints and their
  formatter, admission and hosts (kkt-web, kkt-mcp, kkt-format,
  kkt-admission, kkt-hosts), and the preview's items, which stay in kkt-web
  until 6.3 moves the preview out. It
  also makes F4's change on today's `Rules`: the overview takes the activity
  bucket and window as arguments, and kkt-web passes its own
  `WEB_ACTIVITY_*`. kkt-rules uses them today (`human.rs`), so moving them to
  kkt-web without this would add an arrow from kkt-rules to kkt-web that F4
  removes. M.
- **3.1c** is split in two, in order:
  - **3.1c-1** the operations crates (kkt-clock-system, kkt-backup,
    kkt-container, kkt-diagnostics, kkt-setup, kkt-remote-access,
    kkt-client-config, kkt-service, kkt-watch, kkt-agent-client and the
    program). Their injected tuning values become fields of each crate's own
    `Settings`, which the programs fill. M.
  - **3.1c-2** the rest: the server defaults, and for now the config and
    data paths, to kkt-settings; `LogLevel`, `LogPolicy`, their check and
    the log constants to kkt-logs, outside its `real` feature; and
    `InvalidSetting` to kkt-settings. kkt-web, kkt-admission, kkt-hosts and
    kkt-service cannot use kkt-settings' error, so each gets its own
    `SettingError` here: that part of 3.5a comes forward. M.

Depends on phases 1 and 2 only where an owner crate was split there (for
example, kkt-backup's constants go to kkt-backup, not kkt-backup-os). Not
beside phase 5, which also changes kkt-rules' settings.

3.2 removes kkt-config before 3.3 makes kkt-config-file and 6.3 makes
kkt-web-preview, so an item whose owner does not exist yet goes, for now, to
the crate that holds its code today: the preview's items to kkt-web, and the
config file's folder and name to kkt-settings, beside `parse_file`. They move
on with that code in 3.3 and 6.3. `WEB_SESSION_ID_CHARS` stays with the
sessions in kkt-admission until 6.1 moves them to kkt-web.

**3.2 kkt-config goes.** S. Delete the empty crate and its row in CLAUDE.md,
and fix `.github/scripts/build-outside-crate.sh` if it names it. Depends on
all of 3.1.

**3.3 kkt-config-file and kkt-config-file-os** (F3). M. Parsing the config
file's TOML and editing its text move from kkt-settings to the new logic
crate, with its own `Config` and `ConfigError` and no dependency on
kkt-settings. Reading and writing the config file's and the instructions
file's bytes move from the program into kkt-config-file-os. The program
adapts `parse_file` to kkt-container's new `ConfigReader`. As built, the
program turns a parsed file into kkt-settings' `ServeOptions`: no
`SettingsSource` (see the differences). Depends on 3.2.

**3.4 kkt-machine and kkt-machine-os** (F2). M. `Env` and `Platform` move from
kkt-settings to the new contract crate, with `Machine` and a stand-in behind
`testing`. The paths built from `Env` go to their owners: the config folder
and file to kkt-config-file, the key files to kkt-admission, the data folder
and default database to kkt-setup. `OsMachine` in kkt-machine-os reads the
environment, replacing `src/process.rs`'s `process_env`. As built,
`config_home` stays a method of `Env`; callers pass its result as a plain
optional path to kkt-config-file (decided by Sam, 2026-11-12). Depends on 3.3.

**3.5 kkt-settings only combines its sources** (F2). L, split in three, in
order:

- **3.5a** Each crate's own `SettingError`. 3.1c-2 already gives kkt-web
  (with the preview's code, until 6.3), kkt-admission, kkt-hosts and
  kkt-service theirs; what is left is any other crate that still checks a
  setting with kkt-settings' error. S. As built, nothing was left: every
  crate that checks a setting has its own error, and kkt-service's only
  wrapped kkt-settings' own `resolve`, which 3.5c takes out of its plan.
- **3.5b** Types and checks to their owners: `AccessKey` and `key_refusal` to
  kkt-admission, `check_bind` and `serve_bind` to kkt-hosts. (`LogLevel`,
  `LogPolicy` and `check_dependency_log_level` reach kkt-logs earlier, in
  3.1c-2.) M. As built, `serve_bind` stays in the program and the
  service's bind check is kkt-service's `check_bind` (see the differences).
- **3.5c** kkt-provenance records the run's settings as text, which
  kkt-settings words, and kkt-service's plan takes its own settings. After
  this, only the programs depend on kkt-settings. Each program keeps
  `check_settings`, which turns kkt-settings' values into each crate's
  settings and calls each owner's check, and kkt-keymode keeps its own copy
  of it and of the mapping of `Limits`: no crate may own that mapping, and
  `Limits::check` stays one ordered list so the first bad limit named is the
  same (see the differences). M.

### Phase 4: logging (sequential)

**4.1 kkt-logs and kkt-logs-os** (F7). M. The rollover and pruning rules stay
in kkt-logs, as logic, with the log's settings from 3.1c-2. `RotatingFile`,
the `real` feature, tracing-subscriber and the program's `src/logging.rs`
(starting the log) move to kkt-logs-os. Depends on 3.1c-2. Log files and lines
stay exactly as they are.

**4.2 kkt-log** (F5). M. A new contract crate with `Log`, `Level` and
`testing::CapturedLog`. kkt-mcp and kkt-web take a `Log` and stop using
tracing, and kkt-server's `Parts` take it and hand it on. kkt-logs-os's
`TracingLog` implements it, and the programs wire it in. Depends on 4.1. Not
beside 5.3, 5.4 or 6.x, which change the same constructors in kkt-mcp,
kkt-web and kkt-server. The same events must reach the log, with the same
targets, levels and fields.

### Phase 5: data and core (sequential, the largest)

**5.1 kkt-db is a contract only.** L, split in two:

- **5.1a** The `Legacy` struct (today `crates/kkt-db-sqlite/src/adopt.rs`,
  line 29) moves to kkt-db, with no backend's terms (F15): which set it is,
  which old versions are adopted, which tables go, and the too-old message.
  The adopting stays in kkt-db-sqlite: reading `user_version`, setting old
  tables aside, refusing a database too old to adopt, and
  `SqlitePlace::adopting`. M.
- **5.1b** `Database` goes: its typed read and write become `BackendExt`'s
  provided methods over `Backend`. `migrate` and `pending` become `Evolve`'s
  provided methods. The query checks become `Schema`'s methods.
  kkt-db-sqlite's `SqlitePlace` implements `Backend` directly. kkt-store and
  the programs follow. L.

Depends on 1.8. Must pass `kkt_db::testing::conformance`, and the 0.10.0
fixture must still migrate.

**5.2 kkt-store merges into kkt-rules.** L, split in two, in order:

- **5.2a** Move kkt-store's code into kkt-rules unchanged, as a module,
  `Reads`, `Writes` and their records included, and point every user at the
  new paths. `testing::memory`, `fail_next_write` and `swarms_stay_apart`
  become kkt-rules'. The 0.10.0 migration test moves into kkt-rules' tests.
  `.github/scripts/build-outside-crate.sh` stops naming kkt-store. About
  4,200 lines, all moved; review that it is mechanical
  (`git diff --color-moved`). The schema's migrations move unchanged, and none
  is edited. L in lines, S in substance.
- **5.2b** Remove `Reads`, `Writes` and the copied records: the rules call
  the queries directly, and the queries reach kkt-db directly. M.

Depends on 5.1.

**5.3 kkt-api-agent.** L, split in two:

- **5.3a** The 47 record types the proposal moves (agents, channels,
  messages, memory, lanes, errors, `SwarmId`) move from kkt-rules to the new
  contract crate, with the new `Limits` record (F14). Every user follows. L
  in lines but mechanical.
- **5.3b** `Api` and the per-area traits (`Agents`, `Channels`, `Messages`,
  `Memory`, `Lanes`). `Rules` implements them; kkt-mcp's agent endpoint takes
  them instead of `Rules`. `Identify::avatar` takes kkt-api-agent's `Api` in
  place of `&Rules`, so kkt-identity's arrow to kkt-rules goes, and
  kkt-identity-keymode and the fake product's `FakeIdentify` follow. M.

Depends on 5.2.

**5.4 kkt-api-operator and kkt-api-admin.** L, split in two:

- **5.4a** kkt-api-operator: `Views`, `Changes`, `Lanes` and `Server`, with
  their records. `Rules` implements them. `Views::overview` keeps the bucket
  and window arguments 3.1b gave the overview. M.
- **5.4b** kkt-api-admin: `Admin` and `Swarms`. `Rules` implements them, and
  kkt-mcp's admin endpoint takes them. M.

Depends on 5.3.

**5.5 The default wording** (F9). S. `NoticeWording` moves from
kkt-mcp-formatter into kkt-rules as `DefaultWording`, its own implementation
of its own `OperatorWording`. The programs pass it in, and kkt-server's
wording adapter goes. Depends on 1.6. Can run beside 5.1 and 5.2.

**5.6 The store goes private** (msg 2659). S to M. kkt-rules'
`testing::memory` gives `Rules` through the kkt-api contracts instead of a
`Db`, and the store's records, `Reader`, `Writer` and `Db::read`/`write` go
private. Other crates' test setup goes through the contracts. If the
program's 0.10.0 migration test cannot go through them, a differences row
says what stays public and why. After 6.3. Split in two: **5.6a**
(#476), kkt-rules' `start` records the run and builds the `Rules`, which
the programs pass to kkt-server's `Parts`; **5.6b**, the store going
private, after it.

### Phase 6: the web page and the server (sequential)

**6.1 Web sessions move into kkt-web** (msg 2021). M. kkt-admission keeps the
access decision. kkt-web's `Gate` keeps only the two questions for
kkt-admission, and kkt-server's `AdmissionGate` implements it. Depends on
3.1b. Can run beside phase 5.

**6.2 kkt-web uses the kkt-api traits directly.** M to L. `Source` gives way
to `Api`, `RulesSource` and its copied records go, and `PageLimits` holds the
page's own limits. Depends on 5.4 and 6.1.

**6.3 kkt-web-preview and kkt-api-sim** (F8, F16). M. The preview becomes a
program of its own (kkt-web-preview). `Preview` and `PreviewGate` move to the
new test support crate kkt-api-sim, behind `testing`, and answer `limits`
with their own values. kkt-web's and kkt-mcp's tests use the stand-ins
(fixed clock, seeded entropy, a manual stop) instead of the `-os` crates.
`check-crates.sh` must allow the dev-only preview program to turn on
kkt-api-sim's `testing` (see "What needs Sam"). Depends on 6.2.

### Phase 7: the programs do no machine work (F1)

These change the program's commands, so they collide with each other in
`src/`. 7.1a goes first; after it, 7.1b, 7.2, 7.4, 7.5 and 7.6 can run in
parallel, 7.3 runs alone, and 7.7 follows it.

**7.1 kkt-terminal-os.** L, split in two:

- **7.1a** Output. `Terminal` (output, errors, whether output is a terminal,
  the closed-pipe default with libc) moves out of the program, and every
  command writes its output through it instead of `println!` and
  `eprintln!`. This touches every command, but each change is the same small
  one. M.
- **7.1b** Questions and the login link. Reading a line and whether input is
  a terminal join `Terminal`; `BrowserLink` moves out of the program;
  kkt-setup gains `Lines` and `Prompter`, the questions' rules, and the
  program adapts `Terminal` to `Lines` and `BrowserLink` to kkt-container's
  `LoginLink`. M.

Depends on phase 3.

**7.2 kkt-admission-os.** M. Reading and writing the key files moves out of
the program. kkt and kkt-keymode adapt `KeyFiles` to kkt-admission's
`KeyTexts` instead of reading files themselves. Depends on 3.5 and 7.1a.

**7.3 kkt-setup-os and setup's decisions.** L, split in two, in order,
after 7.1a:

- **7.3a** The setup record, its unfinished note, the data folder, the
  program's path and PATH move to kkt-setup-os. Which server and key a client
  uses, and the record's rules, move to kkt-setup. The Docker folder's
  `.env` and the Tailscale auth key file are read through kkt-container's
  `Host`. The client's key file, which `src/machine/connection.rs` reads,
  is read through the shared key-file provider, kkt-admission-os (msg 2619).
  M.
- **7.3b** Doctor's checks, the status lines, the backup folders, uninstall's
  record and the per-server key file's name move to kkt-setup. M.

**7.4 Agent credential files.** M. They move to kkt-agent-client, which
reads a file's owner and mode and tightens it. kkt-setup's
`credential_file_check` holds the rule. Watch's state file becomes
kkt-agent-client's `StateFile`, which the program adapts to kkt-watch's
`Place`. Depends on 7.1a.

**7.5 The dev programs.** S to M. The fake product's HTTP client moves into
its tests, through kkt-agent-client's `Http`. Its main uses `Terminal` and
`SqlitePlace::create`. kkt-keymode uses the shared key files and checks.
Depends on 7.1a and 7.2.

**7.6 The service file's test fixture.** S. kkt's `src/machine/service.rs`
adapter already hands its writes to kkt-service-os; the Unix permission
fixture that writes files moves into a test-only module, so the
program-machine-work check sees no machine work there. Found by 8.2's
check (msg 2597).

**7.7 The admin name's key binding.** S. `admin_name_line`, `bound_to` and
`bound_in`, which tie an admin name to the hash of its key, move into
kkt-admission's `server_keys`, as the proposal has them (decided item 33's
split of `AdminName`). The name rules stay in kkt-names. Today the program
calls kkt-names' `AdminName::bound_to` and `bound_in` and keeps
`admin_name_line`; no earlier step moved them. After 7.3a and 7.3b (msg
2659).

Done for the phase when kinds.py reports no machine work in any program's
own code.

### Phase 8: finish

**8.1 The map is the code.** M. Regenerate `map-data.js`. Compare it with
the proposal and record any remaining difference in `decisions.md`, with its
reason. Update CLAUDE.md's crate tables to the new layout. Write one
CHANGELOG.md entry for the whole refactor, for crates that depend on kkt by
git. Turn the page's proposed map into the record of the finished refactor,
or retire it (Sam's choice; default below). Depends on everything.

**8.2 The rules become CI checks.** S. Until here, the scorecard reports the
proposal's rules, and nothing fails on them. This step makes CI enforce the
two that the code can now meet:

- **No `real` in a contract crate** (CLAUDE.md's C5): `check-crates.sh` fails
  if a crate that concerns.json marks as a contract has a `real` feature or
  an item behind one.
- **The programs do no machine work** (F1): a CI job runs kinds.py's checks on
  the map built from the code (`generate.py --out`, not the map's currency
  check), and fails on a composition crate that does machine work.

It runs on pull requests into `refactor`, `next` and `main`. Depends on 8.1.

### Phase 9: kkt-settings goes (Sam, 2026-11-12)

**Why.** kkt-settings gathers other crates' concerns under one name, as the
removed kkt-config did. Its `Limits` repeats the defaults that kkt-rules,
kkt-mcp-formatter and kkt-web already state in their own `Settings`, and
tests exist only to keep the two copies equal. Settings are not a domain:
each crate owns its own settings, defaults and checks, and gathering them is
composition (msgs 2773 and 2775).

**The design.**

1. **Defaults, bounds and record lines stay with their owners.** Each
   crate's own `Settings`, `DEFAULT` and check are the only copy. Each
   owner's `Settings` gains `problems()`, every bad setting it holds, and
   `record_lines()`, its own name and value lines for the run record, byte
   for byte as today. `Limits` and its repeated defaults go, with the tests
   that compared the two.
2. **The serve surface is composition**, in no logic crate: which flags and
   config keys `kkt serve` has, how a flag, the file and a default layer, the
   order in which bad settings are reported, and the run record's wording.
3. **kkt-server gets `ServerSettings`**, which holds only the settings of
   what kkt-server builds or passes on, so kkt-server does not become the
   next kkt-settings. Each field is one owner's own `Settings`, or a plain
   value kkt-server uses itself.
   - `DEFAULT` is built only from the owners' `DEFAULT`s.
   - `check` asks its fields' owners for their problems, takes the
     program's problems as plain values, and names the first of all of them
     in today's order. A problem is the setting's name, which of the
     owner's checks found it (a short check name the owner states, such as
     a floor or a bound against another setting), and the rule as the
     owner words it. The checks stay the owners'. kkt-server keeps the
     order as a list of (setting, check) pairs, and knows nothing of the
     program's crates: the program passes kkt-rules', kkt-backup's and its
     own problems in as plain values. So the list names some settings
     kkt-server does not hold, such as `max_lane_chars` or
     `upgrade_backups_kept`, as plain names only: that is the price of one
     order kept in one place, and kkt-server still knows nothing of what the
     names belong to.
   - `record_lines` puts every line of the run record in today's order, by
     name. Its own fields' lines come from their owners' `record_lines()`;
     the program passes its lines in as plain name and value pairs
     (kkt-rules' and kkt-backup's `record_lines()`, and its own). Only the
     allowed origins and hosts, kkt-server's own plain values, are worded
     by kkt-server. With the owners' lines, the text is byte for byte what
     kkt-settings' `RecordedSettings` writes today.
   - `Parts::new` takes `&ServerSettings` in place of today's separate
     settings argument (the formatter's). kkt-server learns nothing of flags,
     files or how they layer.
4. **Settings only the program uses stay with the program.** The test for
   each setting: does kkt-server build or pass on the thing that uses it?
   There is no arrow from kkt-server to kkt-backup or kkt-logs just to carry
   a value.
5. **The programs keep the surface:** the flags (`cli.rs`), the config keys
   (kkt-config-file's `Config`), layering flag over file over
   `ServerSettings::DEFAULT` and the program's own defaults, resolving the key
   files and the database, and mapping each key onto one `ServerSettings`
   field or a setting of the program. kkt-keymode keeps its own, smaller
   surface (the config file only). It repeats about 40 lines of mapping from
   keys to fields, which is what any product would write. The check's order
   and the record's wording exist once, in kkt-server.

**Where each setting goes.** Every field of today's `Limits`, and every
field of `ServeOptions` and `ServeSettings`:

| Setting | Side | Field and owner | Why |
|---|---|---|---|
| `name_pack`, `default_limit`, `max_limit`, `max_body_bytes`, `max_channel_name_chars`, `max_purpose_chars`, `max_description_chars`, `max_note_chars`, `max_server_announcement_chars`, `max_memory_entry_chars`, `max_memory_chars`, `max_lane_chars`, `sends_per_minute`, `registrations_per_minute`, `max_agents`, `default_wait`, `max_wait`, `waiting_grace` (`waiting_grace_seconds`), `member_preview`, `purpose_preview_chars` | program | kkt-rules' `Settings`, which the program puts in `kkt_rules::Startup` | The program builds the `Rules` (5.6a); kkt-server receives them built and never reads their settings |
| `max_response_bytes` | kkt-server | `formatter`, kkt-mcp-formatter's `Settings` | `Parts::new` builds the formatter |
| `web_session_lifetime`, `web_page_messages`, `web_member_excerpt_chars` | kkt-server | `web`, kkt-web's `Settings` | kkt-server builds the page and its sessions |
| (no key) the MCP event streams' timing | kkt-server | `mcp`, kkt-mcp's `Settings` | kkt-server builds the transport; not an operator setting, so it is never recorded |
| `allow_origins` | kkt-server | `allowed_origins`, a plain list | kkt-server's transport uses it |
| `allow_hosts` | kkt-server | `allowed_hosts`, a plain list; kkt-hosts checks it (`AllowedHosts::new`) | kkt-server's transport and page use it |
| `upgrade_backups_kept` | program | kkt-backup's new `Settings` (default 5, at least 1) | The program makes the backups before it opens the database. kkt-backup owns pruning, and its `old_names` already takes the count; both programs need the default and the check, for the record |
| `db` | program | | The program opens the database before the rules start; kkt-server never sees it |
| `bind`, `allow_any_bind` | program | `DEFAULT_BIND` and `DEFAULT_ALLOW_ANY_BIND` in kkt-hosts, beside `check_bind` | kkt-server-listen binds, not kkt-server |
| `instructions` (the file) | program | | The program reads the file. kkt-server uses the text, in `AppConfig::instructions` |
| `access_key_file`, `no_access_key`, the web and admin key files | program | kkt-admission's `AccessKey` and key paths | The program resolves and reads the key files. kkt-server uses what they give: the admissions in `AppConfig` |
| `log_file`, `log_level`, `dependency_log_level`, and the log file's `LogPolicy` | program | the level defaults in kkt-logs, beside `LogLevel` | The program starts the log |
| the config file's path (recorded as `config_file`) | program | | The program reads it |

So `ServerSettings` holds `formatter`, `web`, `mcp`, `allowed_origins`
and `allowed_hosts`. kkt-server uses program-side things only as built
values or plain values: the `Rules` (built with kkt-rules' `Settings`), the
instructions text and the three admissions in `AppConfig`, and the
program's problems and record lines, which `check` and `record_lines`
order.
`AppConfig` keeps the admissions and the instructions text and loses `web`,
`mcp`, `allowed_origins` and `allowed_hosts`, which `ServerSettings` holds.
kkt-server makes `AllowedHosts` from the list `check` passed; 9.2 chooses
how, so long as kkt-hosts' check runs once, in its place in the order.

**Where today's items go.** `DEFAULT_BIND` (and the private
`DEFAULT_ALLOW_ANY_BIND`, now public) to kkt-hosts. `DEFAULT_LOG_LEVEL` and
`DEFAULT_DEPENDENCY_LOG_LEVEL` to kkt-logs. The defaults of the allowed
origins and hosts (none) into `ServerSettings::DEFAULT`. `upgrade_backups_kept`'s
default to kkt-backup's new `Settings`, since kkt-backup has none today.
`InvalidSetting` goes: each owner's error already prints as "name rule", and
the programs add "invalid setting: " (kkt's `Sources::invalid`, kkt-keymode's
own wording), as they do today for kkt-hosts', kkt-logs' and kkt-admission's
errors. `ServeOptions` (with `over`, `gives`, `lane_limit`, `name_pack_name`
and `resolve`), `ServeSettings` and resolving the `AccessKey` move into kkt's
`src/machine/settings.rs`, beside `Sources`, and kkt-keymode keeps its own
smaller copy. `RecordedSettings`' wording goes to kkt-server's
`record_lines`. The commands that read a default without a server (`kkt
status`, doctor, `kkt lanes`, `cli.rs`'s help) read it from its owner:
kkt-rules' `Settings::DEFAULT` for the name pack and lane limit,
kkt-backup's for the backups.

**The order of the check.** Today a setting can be named at two places:
`max_memory_chars` and `web_page_messages` are each checked once against
their floor, among the "must be at least 1" checks, and once more later
(against `max_memory_entry_chars`, and against the page ceiling). That is
why each problem carries the name of the owner's check that found it, and
kkt-server's list holds (setting, check) pairs: those two settings appear
twice, once with their floor check and once with the other. Today's order,
which the list keeps:

1. Before the check, as now, the program's own: the database, the key file
   (`no_access_key`, `access_key_file`) and the name pack, while resolving;
   and in kkt, `dependency_log_level` before the log starts.
2. `bind` or `allow_any_bind` (kkt-hosts' `check_bind`, the program's).
3. `dependency_log_level` (kkt-logs, the program's; kkt-keymode does not
   check it today and does not start to).
4. "must be at least 1", in this order: `default_limit`, `max_body_bytes`,
   `max_channel_name_chars`, `max_purpose_chars`, `max_description_chars`,
   `max_note_chars`, `max_server_announcement_chars`,
   `max_memory_entry_chars`, `max_memory_chars`, `max_lane_chars`,
   `sends_per_minute`, `registrations_per_minute`, `max_agents`,
   `web_page_messages`, `web_member_excerpt_chars`, `upgrade_backups_kept`
   (the program's).
5. `max_response_bytes`, `max_limit`, `max_memory_chars`,
   `web_page_messages`, `web_session_lifetime`, `waiting_grace`,
   `max_wait`.
6. `allow_host` (kkt-hosts).
7. After the check, as now, the program's: the instructions file, then the
   key files.

**What must not change, and the tests that pin it.**

- The config file's keys: kkt-config-file's `every_setting_can_be_given`
  (it leaves `waiting_grace_seconds` and `upgrade_backups_kept` out) and
  `src/machine/settings.rs`' `a_parsed_config_file_gives_each_setting_it_says`
  (all 21 keys), and `tests/serve.rs`' `flags_win_over_the_config_file`.
- The flags and their help, with the defaults it shows: `tests/help.rs`'
  `every_help_page_matches_its_snapshot` (`kkt serve --help` included), and
  `cli.rs`' `serve_args_reads_a_service_files_arguments` and
  `serve_args_refuses_what_serve_does_not_take`. `tests/contract/` pins the
  MCP tools, not the flags; `tests/contract.rs`'
  `descriptions_state_the_live_limits` pins that the tools state the limits
  given.
- Every refusal text and which bad setting is named first:
  kkt-settings' `zero_is_refused_for_every_count`,
  `maximums_may_not_be_below_defaults`,
  `an_avatars_memory_holds_at_least_one_whole_entry`,
  `response_cap_has_a_floor`, `web_pages_have_a_ceiling`,
  `web_sessions_last_at_least_a_second` and
  `grace_is_configurable_with_flag_precedence_and_checked` (they move with
  each check to its owner); `src/machine/settings.rs`'
  `a_config_files_bad_value_is_refused_naming_the_setting` and
  `a_bad_setting_names_the_config_file_only_when_it_set_it`;
  `src/commands/service.rs`' `a_bad_setting_names_the_config_file_that_set_it`,
  `a_key_file_given_with_no_key_is_refused`,
  `without_a_home_the_database_must_be_given` and
  `without_a_home_the_key_must_be_given_or_turned_off`; `serve.rs`'
  `without_a_home_or_a_db_serve_refuses_before_other_settings`; and
  `tests/serve.rs`' `a_bad_config_file_stops_serve_naming_the_setting`,
  `a_bad_flag_is_not_blamed_on_the_config_file`,
  `unknown_name_packs_name_the_flag_or_config_file` and
  `no_access_key_and_a_key_file_together_are_refused`. Today nothing pins
  which setting is named first when several are bad: 9.1 adds that test
  before anything moves.
- The run record, byte for byte: kkt-settings' `record/golden.rs`
  (`a_server_with_every_setting_given_records_them_all` and the seven other
  `a_*_server_*` cases) and `record.rs`' tests of order and wording
  (`default_settings_text_is_fixed`,
  `a_run_records_the_config_file_before_the_key_file`,
  `origin_order_does_not_change_the_text`,
  `host_order_does_not_change_the_text`,
  `a_run_records_whether_any_address_may_be_bound`,
  `a_run_records_the_key_files`,
  `a_run_records_the_admin_key_file_and_name`,
  `a_run_records_its_log_file_and_how_it_is_bounded`). They move to
  kkt-server in 9.2 and keep their expected text, building the program's
  lines from kkt-rules' and kkt-backup's `record_lines()`; each owner's
  `record_lines()` gets a test of its own lines in 9.1.

Existing installs: nothing on disk changes. The config file, the flags, the
service files and the database stay as they are, and a run's record is the
same text, so its hash is too.

**The steps.** Each is one pull request into `refactor`. 9.2 and 9.3
change kkt-server's `Parts::new` and `AppConfig`, so they follow the rule
for the shared constructors: one at a time.

- **9.1 Each owner checks and words its own settings.** S. Parallel: two
  builders, disjoint crates.
  - **kkt-build-b:** first, a test that pins which setting today's code
    names first when several are bad (through `src/machine/settings.rs`'
    `check_settings`, so `bind`, `dependency_log_level`, the limits and
    `allow_host` are all in it, including both places of `max_memory_chars`
    and `web_page_messages`); then kkt-rules' `Settings::problems()` and
    `Settings::record_lines()`.
  - **kkt-build-c:** `problems()` and `record_lines()` for
    kkt-mcp-formatter's and kkt-web's `Settings`, and kkt-backup's new
    `Settings` (`upgrade_backups_kept`, default 5, at least 1, its one
    record line), the one default only `Limits` holds today.

  Each problem carries the setting's name, the owner's name for the check
  that found it, and the rule. Each owner's tests show its `problems()`
  gives the same names and rules as `Limits::check` for its fields, and its
  `record_lines()` the same lines as `RecordedSettings` today. Nothing calls
  them yet.
- **9.2 kkt-server's `ServerSettings`.** M. Constructor work in kkt-server,
  one builder. `ServerSettings` holds `formatter`, `web`, `mcp`,
  `allowed_origins` and `allowed_hosts`; its `DEFAULT`, `check` and
  `record_lines` compose the owners': `check` orders its fields' owners'
  problems and the program's by (setting, check), and `record_lines` orders
  its fields' owners' lines and the program's by name (the golden and order
  tests move here from kkt-settings, and keep their expected text).
  Naming the first of all the problems gives what today's early returns
  give, because no check relies on an earlier one having passed.
  `Parts::new` takes `&ServerSettings`; `AppConfig` loses `web`, `mcp`,
  `allowed_origins` and `allowed_hosts`. The three programs and
  `tests/common` follow; kkt and kkt-keymode build `ServerSettings`,
  kkt-rules' `Settings` and kkt-backup's from `Limits` for now, and
  `Limits::check` gives way to `ServerSettings::check`. After 9.1.

  9.2 also makes Sam's decision on `Parts` (2026-11-12): `Parts` takes an
  `Arc<dyn AdminApi>`, kkt-api-admin's contract, in place of
  `kkt_rules::Rules`, and the programs pass what `kkt_rules::start`
  returns. kkt-server uses the `Rules` only to hand them to its three
  surfaces as `Api`, `OperatorApi` and `AdminApi`; with Rust 1.98.1's trait
  upcasting, the one `Arc<dyn AdminApi>` becomes the other two, so no
  contract changes. `new_server_run`, which turns kkt-provenance's
  `ServerRun` into kkt-rules' `NewServerRun`, moves out of kkt-server to
  whoever calls `start`: for now each program has its own copy (eight
  lines), beside the program's other serving code.
  So kkt-server's normal dependency on kkt-rules goes. Its tests keep a
  dev-dependency on kkt-rules for `kkt_rules::testing`, and are not
  rewritten onto kkt-api-sim. kkt-server keeps kkt-provenance for `Build`.
- **9.3 The programs keep the surface; kkt-settings goes.** M. `ServeOptions`,
  `resolve`, `ServeSettings` and the program's checks move into kkt
  (`src/machine/settings.rs`) and kkt-keymode, which map each key onto a
  `ServerSettings` field or a program setting (kkt-rules' `Settings`,
  kkt-backup's, or its own). kkt-keymode makes no backups: it depends on
  kkt-backup only so that its run record keeps the `upgrade_backups_kept`
  line byte for byte, which is deliberate, and on kkt-names for the name
  pack. Each program passes kkt-rules' and kkt-backup's problems and record
  lines, and its own, to kkt-server's `check` and `record_lines`.
  kkt-settings is deleted, with `Limits` and the tests that compared its
  defaults with the owners' (`each_parts_defaults_are_the_gathered_defaults`,
  in kkt and in kkt-keymode), and the two that compared kkt-rules' record
  lines and refusals with `Limits`' (`the_rules_record_lines_match_the_run_records_own`
  and `the_rules_problems_word_each_refusal_as_todays_check`): the owners'
  own tests, the run record's pinned lines and the 26-step first-refusal
  pin keep those texts. In kkt the surface sits in a private module,
  `src/machine/serve_options.rs`, which `machine::settings` re-exports, and
  `ServeSettings` holds kkt-server's `ServerSettings`, which
  `server_settings` fills. The users of `Limits::DEFAULT` (`cli.rs`, `kkt
  status`, doctor, `kkt lanes`, `tests/common`, `tests/contract.rs`) read
  their owners' defaults. The crate's paperwork goes too: the workspace
  members, CLAUDE.md's row, `concerns.json` and `assembly.json`. Rebase on
  kkt-build-c's change to kkt-config-file (msg 2779), which touches the
  same file. After 9.2. kkt-build-b, after its 11.1; 11.x does not run
  beside it.
- **9.4 Chart what differs.** Folded into 10.4, which charted phases 9 to
  11 together.

### Phase 10: the program's tools become assemblies (Sam, 2026-11-12)

**Why.** kkt's library is about 5,400 lines: `src/commands` (about 2,900)
and `src/machine` (about 2,450), with `src/bin/kkt/cli.rs` (1,067) and
`main.rs` (265) beside it. It composes the server and a dozen small tools
in one package, so kashkeshet depends on 48 kkt crates for unrelated
reasons, and nothing shows which command needs which crate. The tools
split into composition crates by who uses them, beside kkt-server: one for
the operator's commands and one for the agents' commands (msgs 2789, 2794,
2803, 2825). The serving code stays in kashkeshet (msg 2876): kkt-server
is already the server's assembly, and the wiring above it is what the
program is for.

**The rules for the two assemblies.**

- No assembly depends on the other, and none depends on an `-os` crate or
  on kkt-agent-client, which is an effect crate too. Each takes its
  effects in a `Parts` value, as kkt-server takes `Parts`.
- kashkeshet, the `kkt` program, chooses every implementation and injects
  it. It keeps a long dependency list on purpose: the list shows every
  choice in one place.
- The adapters stay in kashkeshet, under the strict rule that the program
  adapts: `src/machine/container.rs`, `service.rs`, `remote_access.rs`,
  `client_config.rs`, `diagnostics.rs`, `credentials.rs` and
  `database.rs`; `src/commands/lanes/disk.rs`;
  `src/commands/watch/server.rs` and `WatchOutput`; `src/terminal/`; and
  `src/machine/connection.rs`' `ConnectionReads`. Each phase-10 step adds
  the small adapters its assembly's own contracts need, in
  `src/machine/`.
- `cli.rs` stays in kashkeshet: the flags are the program's (msg 2794).
  It is 1,067 lines: the clap types of 17 commands (about 1,000 lines),
  `serve_args`, which reads a service file's arguments as a `kkt serve`
  command line, `absolute_flag`, and 2 tests. Nothing in it is an
  assembly's: it decides only how the flags read, and takes each default
  it shows from the crate that owns it. The assemblies take typed
  requests, as `src/commands/*::run` does today (`ServiceAction`,
  `LanesAction`, `WatchTask`, `NewAvatar`, `NewKey` and the rest move with
  their commands; `cli.rs` names them in their new crates).
- Where a command needs a fact about the local server (where it listens,
  which key reaches it, which database it uses, what the setup record
  says), its assembly asks a question through its `Parts`, and kashkeshet
  answers from its own serving code or with kkt-operate's functions, and
  adapts. A question, not a value worked out up front, because today each
  fact is read only by the command that needs it, at the point it needs
  it, and a missing record or a bad config file is reported only then,
  with its own text. That stays.

**The serving code stays in kashkeshet** (msg 2876). There is no
kkt-serve. `src/commands/serve.rs`, `src/machine/settings.rs` (where 9.3
lands the serve surface), `src/machine/database.rs`, the server half of
`src/machine/keys.rs` (`read_server_keys`, `make_default_key`,
`read_key`) and the backup before an upgrade stay in kashkeshet.
`backup_before_upgrade` and the `write_backup` it calls leave
`src/commands/lifecycle.rs` for `serve.rs` when 10.2b moves lifecycle to
kkt-operate, whose `kkt backup` writes through its own `BackupWriter`.
Choosing the database, reading the
keys, resolving the settings, starting the rules and listening is how the
program composes a server from the implementations it chooses. As a
crate it would have had one user, since kkt-keymode must not use it: the
end-to-end tests run against both programs to check two separate
assemblies. So kashkeshet keeps its arrows to kkt-events,
kkt-identity-keymode, kkt-mcp, kkt-web, kkt-names, kkt-config-file and
kkt-api-agent, which only serving needs, and kkt-keymode is untouched by
phase 10.

**kkt-operate** (kind composition, layer assembly). The operator's
commands on this machine.

- Moves in: `src/commands/setup.rs` and `setup/steps.rs` (`OsSteps` and
  `OsSwitchSteps`, which implement kkt-setup's `Steps`, `SwitchSteps` and
  `AdoptSteps`: a composition may), `service.rs`, `lifecycle.rs` (status,
  upgrade, uninstall, backup), `doctor.rs`, `lanes.rs` (not `disk.rs`),
  `key.rs`, and `kkt agent` (list and revoke, from `agent.rs`: it is the
  operator's command on the local database, beside `kkt lanes`, and shares
  its `existing_db`); `src/machine/record.rs` whole (the record, the
  unfinished note, the unrecorded installs, `check_setup`, which `main.rs`
  calls before serving unless `--ignore-setup`); and the
  operator half of `src/machine/keys.rs` (the key finders `find_key`,
  `find_operator_key`, `find_here` and their reads, `selected_key_path`,
  `optional_key_path`, `runs_in_docker`, `local_web_key`,
  `local_operator_key`, `unrecorded_for_keys`, `admin_name_line`,
  `write_admin_name`, `new_key_file`, `write_key_file`).
- `OperateParts`: the contracts its consumers already declare, and a few
  of its own.

  | Field | Contract | Answered by |
  |---|---|---|
  | `container`, `container_configs`, `login_link` | kkt-container's `Host`, `ConfigReader`, `LoginLink` | `ContainerHost`; `ContainerConfigs` (the program's `parse_config`); `TerminalLink` |
  | `service`, `try_bind` | kkt-service's `System`, and the address check `check_bind` takes | `ServiceSystem`; kkt-service-os' `try_bind` |
  | `remote` | kkt-remote-access' `Host` | `RemoteHost` |
  | `clients` | kkt-client-config's `Host` | `ClientHost` |
  | `prober` | kkt-diagnostics' `Prober` | `DiagnosticProber` |
  | `lanes` | kkt-lane-files' `Disk` and `git::Git` | `LaneDisk` |
  | `lines` | kkt-setup's `Lines` | `TerminalLines` |
  | `connection` | kkt-setup's `connection::ConnectionSource` | `ConnectionReads` |
  | `clock` | kkt-clock's `Clock` | kkt-clock-system |
  | `serve` | `ServeFacts` (kkt-operate's): the local server's facts, below | kashkeshet's serving code (`src/machine/settings.rs` and the database it uses), and `cli::serve_args` |
  | `setup_files` | `SetupFiles`: read, write and remove the setup record and the unfinished note; make a path absolute or canonical; this program's path and `kkt`'s | kkt-setup-os |
  | `key_files` | `KeyFiles`: read, read if there, whether there (and its error), whether a file; write a new key, a given key, the admin name | kkt-admission-os, with kkt-entropy-os for new keys |
  | `config_file` | `ConfigText`: read and write the default config file's text, for setup's `allow_hosts` | kkt-config-file-os |
  | `database` | `Databases`: whether a database file is there, open it (a `kkt_rules::Db`, for `CommandLine`), copy it | kashkeshet's `database.rs` |
  | `backups` | `BackupWriter`: write kkt-backup's layout | kkt-backup-os |
  | `output` | `Output`: standard output and error | kkt-terminal-os |

- Depends on (17 kkt crates): kkt-setup, kkt-container, kkt-service,
  kkt-remote-access, kkt-client-config, kkt-diagnostics, kkt-lane-files,
  kkt-backup, kkt-admission, kkt-names, kkt-rules (`CommandLine` and the
  lane checks, and its `Settings::DEFAULT` for the name pack and lane
  limit after 9.3), kkt-api-agent, kkt-api-operator, kkt-config-file,
  kkt-machine, kkt-provenance and kkt-clock. Third-party: tokio (setup's
  steps wait on async work from inside, as today).
- Takes from kashkeshet the arrow to kkt-api-operator, and most uses of
  kkt-setup, kkt-container, kkt-service, kkt-diagnostics, kkt-lane-files
  and kkt-backup, which then stay in kashkeshet only for serving and the
  adapters.

**kkt-agent-commands** (kind composition, layer assembly). The agents'
commands.

- Moves in: `kkt register` and `kkt reclaim` (from `agent.rs`, with
  `NewAvatar`, `lane_refused` and `avatar_error`, which decide the
  wording), `kkt watch` (`run`, `WatchTask`, the timed-out exit code), and
  `kkt connect`; from `src/machine/connection.rs`, `remote_client` and
  `admin_client` (the key read and its wording, and the admin endpoint),
  through the contracts below.
- `AgentParts`:

  | Field | Contract | Answered by |
  |---|---|---|
  | `connection` | kkt-setup's `connection::ConnectionSource` | `ConnectionReads` |
  | `admin_key` | `AdminKey` (kkt-agent-commands'): where the admin key is, given `--key-file` or not | kkt-operate's `selected_key_path` |
  | `admin_path` | The admin endpoint's path, a plain value | kkt-mcp's `MCP_ADMIN_PATH` |
  | `servers` | `Servers`: a client of the server at an address with an optional key, which registers, reclaims and sets a description and reports its own failures (lost, key refused, refused with a code); and kkt-watch's `Server` for an agent or the admin endpoint | kkt-agent-client's `Client`, `Remote`, `AdminRemote`, with `WatchServer` |
  | `credentials` | `Credentials`: reserve a credential file and save to it, read a token file, claim a state file as kkt-watch's `Place` | `credentials.rs` |
  | `key_files` | `KeyText`: read the client's key file | kkt-admission-os |
  | `clients` | kkt-client-config's `Host` | `ClientHost` |
  | `program` | `Program`: `kkt`'s path, for a client's config | kkt-setup-os |
  | `report`, `output` | kkt-watch's `Report`; `Output` for the handle lines | `WatchOutput`; kkt-terminal-os |
  | `clock` | kkt-clock's `Clock` | kkt-clock-system |

- Depends on (6 kkt crates): kkt-setup, kkt-watch, kkt-client-config,
  kkt-names, kkt-machine and kkt-clock. Third-party: futures. The admin
  endpoint's path comes in `AgentParts` (10.3): an arrow to kkt-mcp for
  one constant would add kkt-mcp's build to the assembly's, and changed
  the third-party feature list `cargo tree` reports.
- Takes no arrow from kashkeshet, which still uses each of these crates
  for serving, its adapters or `cli.rs`.

**The local server's questions.** Each assembly declares only what its own
commands ask. Every one is lazy today, and stays so: asked only on the
path that needs it, its error passed on unchanged.

| Assembly | Question | Asked by | Lazy today | Pinned by |
|---|---|---|---|---|
| kkt-operate | `ServeFacts::config(file)`: the config file in effect and what it gives (database, access key, lane limit, name pack) | `kkt agent` and `kkt lanes` (only without `--db`), `kkt backup`, `kkt status` (only for a service), the key finders | Read only by those paths, after the record when the record decides | `tests/agent.rs`' `an_explicit_database_does_not_read_the_config_file` and `the_database_is_found_through_the_config_file`; `lanes.rs`' `the_lane_limit_is_the_config_files_else_the_default`; `tests/setup.rs`' `service_setup_checks_lanes_against_this_machines_config_file` |
| kkt-operate | `ServeFacts::configured_access_key()`: the access key the serve config names, a bad setting said bare | the key finders | Only when the record, the unfinished note and the unrecorded install leave the answer open | `keys.rs`' 16 unit tests (`a_recorded_key_reads_only_the_record`, `with_no_record_the_access_key_reads_only_the_config`, `keys_off_with_an_unfinished_service_read_the_config_twice`, `an_unrecorded_service_with_a_key_reads_no_config`, `the_key_finders_say_a_bad_setting_without_naming_its_file` and the rest) |
| kkt-operate | `ServeFacts::database(given, from_config)`: the database in effect, refused without a home | `kkt agent`, `kkt lanes`, `kkt backup` | After the config read, only when no `--db` | `lanes.rs`' `the_database_is_the_given_one_else_the_config_files_else_the_default` and `without_a_home_the_database_must_be_given`; `tests/agent.rs`' `a_missing_database_is_not_created`; `tests/backup.rs`' `backing_up_a_missing_database_is_refused` |
| kkt-operate | `ServeFacts::bind()`: where the server listens with the default config file | setup's service install and uninstall, only with Tailscale; the unrecorded service | Only when Tailscale serves it, or a service is found with no record | `tests/setup.rs`' `setup_adopts_an_installed_service_with_no_record` for the unrecorded service. Nothing pins the Tailscale paths, which would run `tailscale`: 10.2b adds a test, with stand-in `OperateParts`, that uninstalling a service not shared over Tailscale never asks |
| kkt-operate | `ServeFacts::settings(file, flags)` and `settings_of(args)`: what `kkt serve` would run with, by default and from a service file's arguments, compared as a running server sees them | the unrecorded service | Only when a service is installed and there is no record | `tests/setup.rs`' `a_service_installed_with_other_settings_is_not_adopted` and `setup_adopts_an_installed_service_with_no_record` |
| kkt-operate | `ServeFacts::service_settings(file, flags)` and `check_service(...)`: the settings a service install passes, then `kkt serve`'s whole check on them, which names the config file that set a bad one and reads the key files and instructions | `kkt service install`, `kkt upgrade` of a service, setup | Only for an install | `service.rs`' `a_bad_setting_names_the_config_file_that_set_it`, `a_key_file_given_with_no_key_is_refused`, `without_a_home_the_database_must_be_given`, `without_a_home_the_key_must_be_given_or_turned_off`, `the_plan_names_the_default_config_file_for_bind`; `tests/service.rs` |
| kkt-operate | `ServeFacts::default_access_key()`: the default key file's path (no read) | `kkt key create`, `kkt backup`, the service plan | A path only | `tests/key.rs`, `tests/backup.rs` |
| both | kkt-setup's `ConnectionSource`: the record, else the serve config (address and access key), a missing default key made (connect only), the key checked | kkt-operate: doctor, setup's service install; kkt-agent-commands: register, reclaim, watch, connect | The record first; the config only without one or with `--config`; a key made only by `kkt connect` | `connection.rs`' four unit tests; kkt-setup's `record_refusal_precedes_override_resolution_and_config_skips_the_record` and `read_only_selection_resolves_without_making_or_reading_a_key`; `tests/connect_cli.rs`' `a_wrong_config_name_is_refused_before_a_key_is_created`, `connecting_creates_the_default_key_but_preview_does_not_write_client_config`, `client_defaults_follow_serve_config_and_explicit_overrides_win`, `no_key_config_does_not_create_or_configure_credentials`; `tests/register.rs`' `bad_address_and_key_fail_without_leaking_credentials_or_leaving_files` |
| kkt-agent-commands | `AdminKey::path(given)`: where the admin key is: the record, else the unfinished note, else an unrecorded install, else the serve config | `kkt watch --admin` | Each read only when the one before leaves it open | `keys.rs`' key-finder tests; `tests/watch.rs`' `admin_watch_uses_the_local_server_when_the_clients_were_switched` and `admin_watch_refuses_other_keys_and_agent_options` |

The record itself (read, unfinished, unrecorded) is kkt-operate's own
code, through `SetupFiles`; `recorded_setup` keeps its lazy readers, so
`kkt upgrade` and `kkt uninstall` without a record say what they said
(`tests/lifecycle.rs`' `upgrade_and_uninstall_without_a_record_request_setup`).

Only one fact is asked by both: how a client reaches the local server.
It is kkt-setup's `ConnectionSource`, the question kkt-setup's connection
functions already ask, so neither assembly declares a copy of it, and
kashkeshet's `ConnectionReads` answers both from its serving code.

**Dependency counts, before and after.** Normal dependencies on kkt
crates, from the manifests today, and after phases 9 and 10 from the
proposed map.

| Package | Today | After | What changes |
|---|---|---|---|
| kashkeshet | 48 | 48 | Gains the 2 assemblies. Loses kkt-api-operator to kkt-operate, and kkt-settings in 9.3. Keeps its serving code's crates (kkt-events, kkt-identity-keymode, kkt-mcp, kkt-web and the rest), the 19 effect crates it chooses, kkt-server, and in all 26 crates whose contracts or types its serving code, adapters and `cli.rs` name. The count stays, but each arrow now has one reason: serving, an adapter, the flags, or an assembly |
| kkt-keymode | 25 | 26 | Phase 10 does not touch it: it keeps its own surface and serving. Phase 9 takes away kkt-settings and adds kkt-backup (for the record's `upgrade_backups_kept` line) and kkt-names (the name pack) |
| kkt-fake-product | 23 | 23 | Phase 10 does not touch it: it serves two swarms with its own `Identify` and no settings surface |
| kkt-web-preview | 14 | 14 | Phase 10 does not touch it: it serves the page alone over simulated data |
| kkt-operate | | 17 | New |
| kkt-agent-commands | | 6 | New |

**What must not change, and what pins it.** Every command's output, exit
codes and help, and every install's files.

- Help: `tests/help.rs`' `every_help_page_matches_its_snapshot`, and
  `cli.rs`' `serve_args_reads_a_service_files_arguments` and
  `serve_args_refuses_what_serve_does_not_take`.
- What agents see: `tests/contract/` and `tests/contract.rs`; no step runs
  `UPDATE_CONTRACT=1`.
- Exit codes: `kkt watch --once --timeout`'s 3 (`tests/watch.rs`'
  `the_timeout_ends_it_with_no_library_lines`), `kkt key show --admin`'s
  `NO_ADMIN_KEY_EXIT` (`tests/key.rs`' `show_without_a_key_file_is_an_error`
  and `show_admin_reports_no_admin_key_when_the_access_key_is_named_admin_key`).
- The end-to-end runs against both programs (`KKT_TEST_PROGRAM=kkt` and
  `kkt-keymode`): `tests/e2e.rs`, `admission.rs`, `web_login.rs`,
  `admin.rs`, `watch.rs` and `latency.rs`.
- Serving: `tests/serve.rs`, `kkt.rs`, `startup.rs`, `shutdown.rs`,
  `logs.rs`, `backup.rs` (the backup before an upgrade),
  `migrations.rs` and `database_files.rs`, and `serve.rs`' three unit
  tests.
- Setup, service and lifecycle: `tests/setup.rs` (43 tests, with
  kkt-setup's own), `tests/service.rs`, `tests/lifecycle.rs`,
  `tests/docker_files.rs`, `tests/clients.rs`, `tests/lanes.rs`,
  `tests/lane_files.rs`, `tests/key.rs`, `tests/doctor.rs`,
  `tests/agent.rs` and `tests/backup.rs`; `src/commands/service.rs`',
  `lifecycle.rs`', `doctor.rs`' and `setup/steps.rs`' unit tests, and
  `src/machine/service/tests.rs`.
- The agents' commands: `tests/register.rs`, `tests/connect_cli.rs`,
  `tests/watch.rs` and `agent.rs`' three lane-refusal tests.

Unit tests move with their code where they test values or fakes (the key
finders' read-order tests, the lane refusals, the bad-setting wording).
Those that read real files today (`existing_db`, the service plan) would
need the `-os` crates in an assembly's tests, which CLAUDE.md asks tests
not to turn on. They move instead to kashkeshet's
tests, where they call the assembly through the program's real `Parts`
and keep pinning the same texts. None is deleted.

Existing installs: nothing on disk changes. The record, the config file,
the key files, the Docker folder, the service files, the client configs
and the database stay as they are.

**The steps.** Each is one pull request into `refactor`, with its new
crate's paperwork (workspace members, CLAUDE.md's row, `concerns.json`,
`assembly.json`) and the scorecard.

| Step | What | Size | After | Who |
|---|---|---|---|---|
| 9.1 | Each owner checks and words its own settings | S | (done: #484, #485) | kkt-build-b, kkt-build-c |
| 9.2 | kkt-server's `ServerSettings`; `Parts` takes `Arc<dyn AdminApi>` | M | (done: #494) | kkt-build-c |
| 9.3 | The programs keep the serve surface (kkt, and kkt-keymode's own smaller one); kkt-settings goes | M | (done: #495) | kkt-build-b |
| 10.2a | kkt-operate, with `OperateParts`: the record, the operator's key finders, doctor, lanes, `kkt key` and `kkt agent` | L in lines, M in substance | (done: #496) | kkt-build-c (msg 2896: swapped from kkt-build-b, who took 9.3 and then 11.3) |
| 10.2b | setup, service and lifecycle move into kkt-operate | M | (done: #498, #499, #500) | kkt-build-c, with kkt-build-b taking one of its moves on a branch stacked on it |
| 10.3 | kkt-agent-commands, with `AgentParts`: register, reclaim, watch, connect | M | (done: #497) | kkt-mt-step1 |
| 10.4 | `main.rs` builds the two `Parts` in one place; CLAUDE.md's tables; chart what differs (9.4 folds in here) and the scorecard | S | (done: #502, and this plan's closing pull request) | kkt-mt-step1 |

10.2a comes before 10.2b, not beside it: setup calls lanes' checks and
import and doctor's server fix, lifecycle calls lanes' `existing_db` and
the key finders, and all of them read the record. So 10.2a makes the crate
with the shared parts, and 10.2b adds the commands that build on them.
10.3 touches only the agents' commands and `connection.rs`' clients, so it
runs beside both. 11.x (kkt-db's test stand-in, kkt-build-b) must not run
beside 9.3. 9.3, 10.2a, 10.2b and 10.3 all change `main.rs`, a few
lines each: rebase, not conflict.

10.1 was dropped (msg 2876): it was kkt-serve, and the serving code now
stays in kashkeshet. The other steps keep their numbers.

### Phase 11: kkt-db's test stand-in (Sam, 2026-11-12)

**Why.** kkt-rules' tests ran on kkt-db-sqlite's in-memory database, an
effect crate's, for speed (5.3b's row). Sam did not accept the exception
(msg 2805): a fast stand-in in kkt-db's `testing` replaces it, and the
program's tests run what matters on the real backend.

| Step | What | Pull request | Who |
|---|---|---|---|
| 11.1 | kkt-db's `testing::FastPlace`: a stand-in on SQLite in memory with its own translation, checked against `MemoryPlace` by a differential test | #487 | kkt-build-b |
| 11.2 | kkt-rules' tests run on `FastPlace`; kkt-rules no longer depends on kkt-db-sqlite; the program's `tests/store_on_sqlite.rs` runs the store's whole surface, in two swarms, on kkt-db-sqlite (`testing::swarms_stay_apart_in`) | #490 | kkt-build-b |
| 11.4 | `kkt_rules::testing` names no store type: the program's tests use `every_store_read`, `rules_on` and `database` | #492 | kkt-build-b |
| 11.3 | A chosen 50 of the rules' scenarios (`testing::scenarios::run_all`) run on kkt-db-sqlite from the program's `tests/rules_on_sqlite.rs`, with 3 dropped where a real-backend test already covers them | #493 | kkt-build-b |

11.4 merged before 11.3, which needed its narrower `testing`.

### Survey: outside tools behind contracts (Sam's review of the map, 2026-11-12)

**Decided by Sam, 2026-11-12 (msg 3041).** A third-party dependency is always
split. A contract crate named for the scope declares what kkt needs, in the
project's terms. A wrapper crate named for the tool implements that contract
directly. The program only builds the wrapper and injects it; it holds no
adapter. This holds even with one implementation. Step 12.1, remote-access,
is the pilot (kkt-build-c). Sam has seen the table below (msg 3050): the
other conversions start once 12.1 merges and sets the pattern.

**Sam's answers to the survey's questions (2026-11-12, msg 3047).**

1. **Libraries.** RUST.md rule 7 already answers it: every third-party
   crate is wrapped by project crates, and its types stay inside them;
   language-shaping crates are exempt. No new rule.
2. **Programs that come with the system count**, the browser opener
   included. The point is easy testing as well as swapping. So the opener
   splits.
3. **What is inside a tool's wrapper: option (a)** (msg 3050). One wrapper
   crate per tool holds both the tool's knowledge and the code that runs
   it. The running is behind `real`, and the knowledge is tested over a
   private runner and a private fake. There is no shared "run a program"
   contract and no shared runner crate. Sam's reason: the scope's contract
   ("you can use a container") is the abstraction other developers use
   without thinking about what is behind it. Only whoever works on the
   wrapper thinks about the tool. A shared process crate would hand every
   developer a new option and a new concern, far from what kkt does.
4. **No renames.** kkt-entropy-os and kkt-logs-os keep `-os`: `-os` says
   the work is the operating system's, and a tool's name is for a third
   party such as Tailscale.
5. **The five client config formats are not five implementations.** Only
   the `claude` command gets a wrapper.

**What Sam found.** kkt-remote-access, a logic crate, declared
`Host::run(args)`: "run `tailscale` with these arguments". kkt-remote-access-os
ran the command and implemented nothing, and the program held a 17-line
adapter between them. A contract phrased in the tool's terms abstracts
nothing (ARCHITECTURE.md 1.2.2, 2.3.2, 3.1.2.1, 3.2.3.1, the `vpn` and
`vpn-tailscale` example).

**Why the code had the other shape.** The proposal's third rule: a contract
used by two crates or fewer is declared by each consumer, and the program
adapts the implementation to it (msgs 2019, 2020). Sam also accepted effect
crates that depend on nothing (2.1 to 2.4 in "Awaiting Sam", group c).
Sam's ruling replaces that rule for third-party dependencies; 12.1 changes
its text.

**The yardstick.** For each pair: is what the effect wraps a third-party
tool or library? Here a tool is a program kkt runs: `tailscale`, `docker`,
`launchctl`, `systemctl`, `git`, `claude`, and `open` or `xdg-open` for the
browser. A library is one from crates.io. The machine's own files,
environment and terminal, reached through Rust's standard library, are not
third-party.

| Pair | What the effect wraps | Third-party? | Contract phrased in | Tool knowledge in the logic crate | The program's adapter | Recommended | Wrapper's name |
|---|---|---|---|---|---|---|---|
| remote-access | The `tailscale` command | Yes | The tool's terms: `Host::run(args)` | All of it: `tailscale status --json`, the serve arguments, the failures (275 lines, 342 of tests) | 17 lines: copies three fields, and words a failure to run Tailscale with advice | **Split** (done in 12.1 (#507)). The contract becomes preparing, serving, checking and stopping the sharing of a server, with their records | kkt-remote-access-tailscale |
| container | The `docker` and `docker compose` commands, and the managed folder's files | Yes | The machine's terms: run a command, read and write a file, say a line | All of it: the Compose files, every Docker command and its output, and the Tailscale sidecar's status (about 1,900 lines, 2,500 of tests) | 63 lines: copies fields | **Split** (done in 12.2 (#510)) (Sam: at least this one). The contract is what kkt-operate and kkt-setup ask: provision, status, up, down, backup, lanes in and out, upgrade, `existing`, the key checks, with `Folder` and the records. The sidecar's Tailscale status (54 lines) goes with the Docker wrapper, which runs it | kkt-container-docker |
| service | launchd (`launchctl`, the plist) and systemd (`systemctl`, the unit file) | Yes: programs kkt runs, though they come with the system | The machine's terms: files, commands, the user ID | All of it, both systems in one crate (launchd 467 lines, systemd 248), chosen by `Manager::for_this_os` | 48 lines: copies fields | **Split, with two wrappers** (done in 12.3 (#509)): the case 1.2.2 names. The program picks one by platform. The plan (what a service runs, from its settings) and the bind check use no tool; they stay logic, as functions on the contract's own values (3.2.5) or in kkt-operate | kkt-service-launchd, kkt-service-systemd |
| lane-files | The machine's files, and the `git` command | Yes, for git | `Disk` in the project's terms (it already asks for the folder's commit); `git::Git` in git's: run git with these arguments | git's meaning: expected absences, the dirty suffix, the wording (56 lines). Sam accepted keeping it there (2.6); his ruling reverses that | 45 lines: copies fields | **Split** (done in 12.5a (#512)). The contract is the commit a lane folder is at, in the project's terms; the wrapper runs git and holds its meaning. The lane files stay behind kkt-lane-files' `Disk`, with the program's adapter (msg 3120) | Contract: kkt-lane-commit. Wrapper: kkt-lane-commit-git |
| agent-client | rmcp and reqwest, reaching kkt's MCP endpoints | Yes | The project's terms: kkt's tools (register, reclaim, describe, read, wait) | None: kkt-watch and kkt-agent-commands each declare a small client contract of their own | `WatchServer` (about 105 lines), `McpServers` (about 85) and the credential files (about 75, which also apply kkt-setup's credential-file rule): copy records, map failures | **Split** (done in 12.4 (#508)). A contract crate for a client of kkt's server, which kkt-watch, kkt-agent-commands and the fake product's tests depend on in place of their own; the rmcp client implements it. The credential and state files are the machine's files, and stay where they are | kkt-agent-client-rmcp (the contract keeps the name kkt-agent-client) |
| diagnostics | HTTP and kkt's MCP handshake, through reqwest | Yes | The project's terms: read a key file, probe an address and get checks | None. The effect crate reads the server's answers (2.5, accepted) | 44 lines: copies fields, passes the doctor's constants | **Split** (done in 12.5b (#514)). The contract (`Prober`, `KeyFile`, `Check`) moves from kkt-diagnostics, the logic that uses it, into a contract crate; the reqwest prober implements it, and the adapter goes | Contract: kkt-probe. Wrapper: kkt-probe-reqwest |
| client-config | Five AI clients' config files, and Claude Code's `claude` command | Yes for `claude`. The five file formats are other programs' formats, which kkt-client-config implements as external contracts (2.3); the files are reached through the standard library | The project's terms for the files (read, write, rename, restrict); the tool's for `claude(args)` | The five formats and where each client keeps them: the crate's concern | 45 lines: forwarding only | **Split the `claude` command only** (done in 12.5c (#511)). A contract crate in the project's terms, "remove kkt's server; add it as the client's own entry", implemented by a wrapper. The files stay behind `Host`, with the program's adapter (msg 3084): they are the machine's own, outside Sam's ruling, and an effect crate may not implement a logic crate's own contract. The five formats are not five implementations (Sam, answer 5) | Contract: kkt-client-command. Wrapper: kkt-client-command-claude |
| terminal | The terminal, and the browser opener (`open`, `xdg-open`) | The terminal: no. The opener runs programs the system provides | Each consumer's own terms: `Lines` (kkt-setup), `LoginLink` (kkt-container), `Output` (kkt-operate, kkt-agent-commands), `Report` (kkt-watch) | None | 20 to 30 lines each | **Split the opener; leave the terminal** (done in 12.5d (#513)). Programs that come with the system count (Sam, answer 2). The opener's contract is in the project's terms (show a link, open it in a browser if asked); its wrapper runs `open` or `xdg-open`. It is about 30 lines. The terminal is the machine's own. The Docker wrapper, which opens the link, is built with the browser (`Docker::new`); the container contract's `LoginLink` keeps the terminal's half (msg 3115) | Contract: kkt-browser. Wrapper: kkt-browser-os, since the work is the operating system's (answer 4) |
| admission | The machine's key files | No | The project's terms (`KeyTexts`, kkt-operate's `KeyFiles`, kkt-agent-commands' `KeyText`) | None | About 40 lines that word each read error with its setting's name, which each program does its own way; the rest forwards | **Leave**: not third-party | (none) |
| setup, backup, config-file | The machine's files: the setup record and note, `PATH`; the backup folders; the config and instructions files | No | The project's terms, as kkt-operate declares them (`SetupFiles`, `BackupWriter`, `ConfigText`) | None | 10 to 43 lines each, forwarding | **Leave**: not third-party. An effect crate may not depend on an assembly, so these adapters stay in the program unless the contracts move into kkt-setup and kkt-backup | (none) |

**Already split.** These implement a project contract directly, and the
program only injects them:
- kkt-db-sqlite (kkt-db's `Backend`, over rusqlite);
- kkt-clock-system (`Clock`, over tokio's timer);
- kkt-entropy-os (`Entropy`, over rand);
- kkt-logs-os (kkt-log's `Log`, over tracing);
- kkt-machine-os (`Machine`, over the standard library: not third-party).

They keep their names (Sam, answer 4): `-os` says the work is the
operating system's. A tool's name is for a third party, such as Tailscale
or Docker. launchd and systemd come with the system, but there are two of
them, so their wrappers take their names to tell them apart (1.2.2).

**Third-party libraries outside these pairs.** RUST.md rule 7 already
covers them (Sam, answer 1). Every third-party crate is wrapped by project
crates, and its types stay inside them: for example axum in kkt-web, kkt-mcp,
kkt-server and kkt-server-listen; rmcp in kkt-mcp; askama in kkt-web;
chrono in kkt-clock; and toml in kkt-config-file. Language-shaping crates
(serde, tokio, futures, thiserror) are exempt.

**What changes in the rules and checks.** 12.1 makes the first changes;
the later steps repeat its pattern.

- **The proposal's third rule.** Its new text is Sam's ruling. `decisions.md` quotes it, and the plan's differences table marks 2.1 to 2.4 and 2.6 as overtaken.
- **Kinds** (`concerns.json`, `proposed.json`, `kinds.py`). The scope crates become contracts: kkt-remote-access, kkt-container, kkt-service, kkt-agent-client, and a new crate for the lane folder and one for the probe. A contract may compute on its own values but not run or wire anything (`kinds.py`'s `beyond_values`). check-crates.sh's check 6 keeps `real` out of a contract crate.
- **Each wrapper is an effect crate** (Sam, answer 3). It has a `real` feature around the command or connection it makes, and the tool's knowledge is tested inside it, without `real`, over a private runner and fake. A wrapper thus holds logic as well as its effect, which sits against ARCHITECTURE.md 4 (one kind per module) and 4.5.2 (complex adapter logic as its own logic module). `refactor-merge.md` notes this for Sam's later edit of the principles.
- **Stand-ins.** Each contract crate has its `testing` stand-in (3.2.4). kkt-operate's and kkt-agent-commands' tests use those stand-ins in place of the fakes they build now.
- **No change to** check-crates.sh's checks 1 to 5, the release guard or the third-party list. check-programs loses each adapter that goes.
- **The shared crates' public surface changes for kkt-pro**, which needs a CHANGELOG entry.
- **Existing installs: none expected.** The Compose files, service files, commands and their output stay byte for byte. The tests that pin them move with the code.

**Steps.** Each step is one pull request into `refactor`. 12.1 sets the
pattern; the rest start once it merges (msg 3050). Who takes which is a
proposal: each builder is reviewed by the other model, as through phases 9
to 11 (kkt-mt-step1's code by kkt-review-b since msg 2273, its plan and map
by kkt-review-373).

| Step | What | Size | Builds | Reviews |
|---|---|---|---|---|
| 12.1 | remote-access: contract and kkt-remote-access-tailscale; the program's adapter goes; the rule's new text | S | kkt-build-c (#507, merged) | kkt-review-373 |
| 12.2 | container: contract and kkt-container-docker, with the Docker logic, its tests and the sidecar's status | L | kkt-build-c (#510, merged) | kkt-review-373 |
| 12.3 | service: contract, kkt-service-launchd and kkt-service-systemd | M to L | kkt-build-b (#509, merged) | kkt-review-b |
| 12.4 | agent-client: the client contract, kkt-agent-client-rmcp; kkt-watch and kkt-agent-commands use the contract | M | kkt-mt-step1 (#508, merged) | kkt-review-b |
| 12.5a | lane-files' git: kkt-lane-commit and kkt-lane-commit-git | S | kkt-build-b (#512, merged) | kkt-review-b |
| 12.5b | The probe: kkt-probe and kkt-probe-reqwest | S | kkt-build-b (#514, merged) | kkt-review-b |
| 12.5c | The `claude` command: kkt-client-command and kkt-client-command-claude | S | kkt-mt-step1 (#511, merged) | kkt-review-b |
| 12.5d | The browser opener: kkt-browser and kkt-browser-os | S | kkt-mt-step1 (#513, merged) | kkt-review-b |
| 12.6 | Chart what differs, and the scorecard | S | kkt-mt-step1 (this pull request) | kkt-review-373 |

12.2, 12.3 and 12.4 run side by side after 12.1: three builders, each on
its own pair. 12.2 and 12.3 each change one field of kkt-operate's
`OperateParts`, and 12.4 changes kkt-agent-commands' `AgentParts`; each
change is a rebase, not a conflict. The small splits follow each builder's
larger step. 12.5d touches kkt-container's `LoginLink`, so it follows 12.2.

### Phase 13: one crate for a contract with one implementation

From the contract audit (#516, `contract-audit.md`, its facts checked by
kkt-review-373 in msg 3153). **Sam took it on 2026-11-12 (msg 3164):** the
six merges go ahead, kkt-client-command merges too, and the `Machine` trait
goes. His answers are under "Questions for Sam" below.

**Why.** Phase 12 split every third-party dependency into a contract crate
and a named wrapper. The audit asked, for each contract crate, whether it
is needed, and found six that have one implementation and one or two
consumers. Merging each with its wrapper is what 3.3.2 describes. In Rust
the other way to drop a contract crate, a trait the consumer declares,
always costs a forwarding adapter in the program, because a type meets a
trait only through an `impl` beside one of them
(`refactor-merge.md`'s sixth note for the principles).

**The target shape: a merged crate.** One crate, named for the scope,
holds:
- the contract in the open: the trait and its records, always compiled,
  and its stand-in behind `testing`;
- the implementation behind `real`, which only programs turn on
  (check-crates' check 2).

Consumers depend on it without `real`, so they cannot name the
implementation: it does not exist in their build, and CI builds each crate
alone. The program turns `real` on and injects the implementation as it
is, with no adapter. The tool's knowledge stays testable without `real`,
over a private runner and fake, as phase 12 made it. Phase 12 compiled a
whole wrapper under `cfg(any(feature = "real", test))`. In a merged crate
that cfg sits on the private modules only, since the open contract must
compile without `real`.

**The wrapper's dependencies become optional, behind `real`.** Otherwise a
consumer that needs only the contract gains the tool's whole dependency
graph, and the map shows new arrows from the contract. For example,
kkt-setup needs only kkt-container's records, and must not gain what
kkt-container-docker uses. Per merge:
- 13.1: kkt-config-file, kkt-machine, serde_json and thiserror, from
  kkt-container-docker;
- 13.2: serde and serde_json, from kkt-remote-access-tailscale;
- 13.3: kkt-browser becomes optional in the merged kkt-container, since
  only `Docker::new` takes a browser;
- 13.4: nothing (kkt-lane-commit-git adds no dependency);
- 13.5: reqwest and serde_json, from kkt-probe-reqwest;
- 13.6: rmcp and reqwest (already optional), tokio and serde_json;
  futures and kkt-clock stay normal.

A dependency that the open contract itself uses stays a normal one. A
merged crate's `real` turns on its optional dependencies, but never
another crate's `real` (check 2): the program does that, as it does today.

**The six merges.**

| Contract crate | Merges with | Consumers today | Step |
|---|---|---|---|
| kkt-container | kkt-container-docker | kkt-operate (the trait), kkt-setup (records and file names) | 13.1 |
| kkt-remote-access | kkt-remote-access-tailscale | kkt-operate | 13.2 |
| kkt-browser | kkt-browser-os | kkt-container-docker (kkt-container, after 13.1) | 13.3 |
| kkt-lane-commit | kkt-lane-commit-git | kkt-lane-files; kkt-operate passes it on | 13.4 |
| kkt-probe | kkt-probe-reqwest | kkt-diagnostics; kkt-operate passes it on | 13.5 |
| kkt-agent-client | kkt-agent-client-rmcp | kkt-agent-commands (`Servers`), kkt-watch (`Watched`) | 13.6 |

**What stays split.**
- kkt-service: two implementations, launchd and systemd.
- kkt-api-agent, kkt-api-operator, kkt-api-admin, kkt-db, kkt-identity,
  kkt-clock, kkt-entropy and kkt-log: several implementations, or more than
  two consumers (3.3.3, 3.3.4).
- kkt-agent-files-os, the machine's own files.

**The two facts sage-cardinal asked to have stated** (msg 3150; checked by
kkt-review-373, msg 3153):
- **kkt-agent-client has two consumer crates:** kkt-agent-commands and
  kkt-watch.
  - The fake product's tests use only kkt-agent-client-rmcp's `Http` and
    `HttpReply`, with `real` on, not the contract. After 13.6 they depend
    on kkt-agent-client with `real`, and the documented exception names it
    again.
  - The program's `tests/register.rs` dev-depends on the contract only to
    bring `Watched` into scope, to call it on rmcp's `Remote`.
- **kkt-container's contract is runtime-neutral, with three values that
  belong to the Docker install.** Its 16 operations, its records,
  `ConfigReader` and `LoginLink` name no Docker or Compose command, file or
  format; a Podman runtime running the same Compose files could implement
  them. The three values are:
  - the key-file names `docker-web-key` and `docker-admin-key`, which
    existing installs fix and kkt-setup reads;
  - `NO_ADMIN_KEY_EXIT`.

  So the case for merging rests on there being one implementation (3.3.2),
  not on the contract being Docker's own.

**Questions for Sam, with his answers** (msg 3164). He also confirmed the
six merges, each into one crate named for the scope, with the contract and
stand-in always compiled and the tool behind `real`. This revises his
earlier ruling of 2026-11-12.
1. **kkt-client-command:** merge it with kkt-client-command-claude, or keep
   it split? It has one implementation today. A second is plausible, since
   other agent clients configure MCP servers through their own command
   line. Two crates' tests use its stand-in. **Answer: it merges too**
   (step 13.7).
2. **kkt-machine's `Machine` trait:** only `main.rs` calls it, and its
   stand-in `FixedMachine` is used nowhere. Recommended: kkt-machine-os
   gives `env()` and `platform()` as plain functions. kkt-machine keeps
   `Env` and `Platform`, which five crates use. **Answer: the trait and
   its unused stand-in go; `Env` and `Platform` stay** (step 13.8).
3. **The rule:** the merges revise Sam's ruling of 2026-11-12. Suggested
   text: a third-party tool or library with one implementation is one crate
   named for the scope, its contract in the open and the implementation
   behind `real`. Several implementations, or a contract many crates
   share, keep a contract crate with named implementations, as kkt-service
   and kkt-db do. Either way the program injects the implementation, with
   no adapter. **Answer: this wording goes into the project's CLAUDE.md.**
   Sam will settle the principles' own text with sage-cardinal separately.

**What changes in the rules and checks** (step 13.0, before any merge).
- **CLAUDE.md's "Third-party tools and libraries" paragraph:** the rule
  above, in Sam's words.
- **CLAUDE.md's "Features" paragraph:** "a contract crate never has
  [`real`]" stays for contract crates, and a sentence adds the merged kind.
  - kkt-agent-commands' "must not": "depend on a contract's implementation
    (such as kkt-agent-client-rmcp)" becomes "turn on another crate's
    `real`".
  - The crate table gains the merged crates' rows, and loses the wrappers'
    rows.
- **A new kind, "contract and effect"** (`concerns.json`, `proposed.json`,
  `kinds.py`). It holds a contract in the open and its one implementation
  behind `real`.
  - **How it differs from "mixed".** `kinds.py`'s "mixed" (a contract or
    logic together with a real effect) is a break, and stays one. The new
    kind differs in two things that must hold together: the effect is the
    implementation of the crate's own contract, and only programs turn its
    `real` on (check 2). A crate that fails either is still "mixed".
    `check()`'s "mixed" branch does not change; the new kind gets its own.
  - `kinds.py` checks that the items outside `real` only compute on their
    own values (today's `beyond_values` test for a contract), that I/O
    dependencies are only optional ones behind `real`, and that the crate
    has a `real` feature.
  - **Data the check needs, which the map does not record yet.**
    - **Each slot's feature.** `beyond_values` flags every slot outside
      `testing::` that takes a trait. After 13.1 and 13.3,
      `Docker::new(Box<dyn Browser>)` sits behind `real` in kkt-container,
      but today's slot record (`Docker::new`, `browser`, no feature) cannot
      say so. generate.py records each slot's feature, and `beyond_values`
      skips slots behind `real`.
    - **Which third-party dependencies are optional, and under which
      feature.** The contract check fails a crate with an I/O dependency.
      A merged kkt-agent-client keeps reqwest, rmcp and tokio, behind
      `real`. generate.py records each dependency's optional feature, and
      the new kind allows I/O dependencies only there.
  - check-crates.sh's check 6 (no `real` in a contract crate) keeps
    applying to "contract" crates only, so its code does not change; its
    comment names the new kind.
  - Check 2 (only programs turn on `real`) is what keeps consumers off the
    implementation, and does not change.
- **`progress.py`:** a dependency on a merged crate without `real` is not
  a break of "depends on the effect crate". Only one that turns `real` on
  is, apart from programs.
- **The proposal:**
  - each merge charted, with the wrapper's items moved under the scope's
    name;
  - the six kind edits;
  - the arrows (the wrappers' arrows to their contracts go);
  - 12.1's rule text, revised.
- **`real-exceptions.json`:** the fake product's entry names
  kkt-agent-client again (13.6).

**The map: a merged crate's two kinds of dependency** (built in 13.9).
Sam wants the map to show which dependency on a merged crate is on its
contract only (built without `real`), and which is on its implementation
(turning `real` on). The data already holds what the drawing needs: each
arrow records the features it turns on and whether it is optional, and
each item whether it is behind `real`. (The kind's check needed more data;
see 13.0 above.)

- **The box,** as built: split left and right, not top and bottom as first
  proposed. Every arrow into a merged crate comes from a layer above it,
  so with the parts stacked, an arrow to the lower part would cross the
  upper one. Side by side, each part has its own top and bottom edge.
  - The left part is the contract: the crate's name and its subtitle.
  - The right part is a tinted strip labelled `real`: the implementation.
- **The arrows:**
  - An arrow that does not turn `real` on (a consumer's grey "uses") meets
    the contract part.
  - An arrow that turns it on (the program's amber "wires", or the fake
    product's dashed test arrow) meets the implementation part.
  - The merged crate's own optional dependencies, which only `real` turns
    on, leave from the implementation part; its others from the contract.
- **The crate's own implementation:** a small hollow green triangle on the
  divider, pointing from the implementation at the contract. An
  implementation of a crate's own trait is not drawn as an arrow, so a
  merged crate would otherwise show no "implements" at all.
- **The panel** lists the two parts' items, and which crates include each
  part ("Its two parts").
- **generate.py:** nothing. map.js reads the crate's kind, each arrow's
  features and whether it is optional.
- **page-test.js:** checks that a merged crate's box has its
  implementation part, that a consumer's arrow and the program's meet
  different parts, and that the panel lists both parts.

**Which way the arrows point** (Sam, msg 3155; built in 13.9). The map draws "A depends on B" as A → B. Sam reads a dependency the
other way: B is included in A, so the arrow runs from the thing to where
it goes. Proposed as the default:

- **uses** (grey): from the crate that is used to the crate that uses it.
- **wires** (amber): from the implementation to the program that builds
  it.
- **implements** (green): unchanged, from the implementer to the contract.

The data does not change (`edges` keep `from` as the dependant), only the
drawing. For a merged crate this gives Sam's own picture: grey from the
contract part out to each consumer, amber from the implementation part out
to the program, and the green mark inside.

**One case needs a rule:** an arrow with several roles whose directions
now differ. The only one is "implements" together with "uses": a wrapper
that implements its contract and uses its records. Proposed: such an arrow
keeps the "implements" direction, with its "uses" stroke beside it, since
using the contract's records is part of implementing it. A program's arrow
that both uses and wires a crate reverses both strokes the same way, so it
needs no rule.

**What else reads backwards today, and would change with it:**
- **The legend and the role words** read as "A uses B" and "A wires B".
  They become "used by" and "built by", with "implements a contract of"
  unchanged.
- **The help text** ("Arrows point from a crate to what it depends on",
  map.js and the page's hint in index.html) says the new direction.
- **The key's "Parallel lines"** (several roles, with one head shaped by
  the first role) also says which way that head points.
- **The edge panel's title,** today "A → B" for A depends on B: drawn in
  the arrow's direction, with "used by", "built by" or "implements" between
  the names.
- **The proposed view's lists of changed arrows** ("A → B") follow the same
  wording.
- **The crate panel** keeps its two lists, renamed for the new reading:
  "Includes" for "Depends on", and "Included in" for "Used by". Each keeps
  its count and code-or-tests marks.

**What already reads Sam's way:** the thin dotted injection lines run from
the program to the crate it injects into, and the assembly flows read in
build order, so neither changes.

**Out of scope:** the command-line scripts' text (`progress.py` and
`check-programs.py` print "A -> B" in Cargo's sense). They can change
later, if Sam wants the same reading there.

**What the code needs:**
- **map.js:** draw a "uses" or "wires" stroke from `to` to `from`; the
  multi-role rule; the wording above. The arrowheads already use
  `auto-start-reverse`, so no new markers are needed.
- **index.html:** the hint.
- **README.md:** the arrows' section.
- **page-test.js:** a check that a "uses" arrow's head is at the user.
  It also picks the "logic" kind in the Crates menu by name: since 13.0
  and 13.6, "contract and effect" is listed before it.
- **generate.py:** nothing.

**Steps.** One pull request each into `refactor`. Each
merge moves code and tests unchanged, keeps every message, file and
command, and charts its own proposal entries. Builders as in phase 12: the
crate's builder merges it; kkt-mt-step1's code is reviewed by
kkt-review-b, and its plan and map work by kkt-review-373.

| Step | What | Size | Builds | Reviews |
|---|---|---|---|---|
| 13.0 | The rule text, the new kind, generate.py's slot features and optional dependencies, kinds.py, progress.py, check-crates' comment, CLAUDE.md, the proposal's rule | M | kkt-mt-step1 | kkt-review-373 |
| 13.1 | kkt-container merges with kkt-container-docker | M | kkt-build-c (this pull request) | kkt-review-373 |
| 13.2 | kkt-remote-access merges with kkt-remote-access-tailscale | S | kkt-build-c | kkt-review-373 |
| 13.3 | kkt-browser merges with kkt-browser-os | S | kkt-mt-step1 | kkt-review-b |
| 13.4 | kkt-lane-commit merges with kkt-lane-commit-git | S | kkt-build-b | kkt-review-b |
| 13.5 | kkt-probe merges with kkt-probe-reqwest | S | kkt-build-b | kkt-review-b |
| 13.6 | kkt-agent-client merges with kkt-agent-client-rmcp | M | kkt-mt-step1 | kkt-review-b |
| 13.7 | kkt-client-command merges with kkt-client-command-claude (Sam's answer 1) | S | kkt-mt-step1 | kkt-review-b |
| 13.8 | kkt-machine's `Machine` trait and `FixedMachine` go; kkt-machine-os gives `env()` and `platform()` as plain functions (Sam's answer 3) | S | kkt-mt-step1 | kkt-review-b |
| 13.9 | The map: split boxes and the arrows' direction (map.js, map.css, index.html, README.md, page-test.js) | M | kkt-mt-step1 | kkt-review-373 |
| 13.10 | Chart what differs, and the scorecard | S | kkt-mt-step1 | kkt-review-373 |

13.0 comes first. 13.1 to 13.8 can then run side by side: each touches its
own crates, and the shared files (Cargo.toml, CLAUDE.md, the proposal) merge
as in phase 12. 13.3 builds on kkt-container's browser field, so it follows
13.1. 13.9 needs at least one merged crate to draw.

**13.7's optional dependencies:** kkt-client-command-claude adds none
beyond the contract, so nothing becomes optional. **13.8 changes no
kind:** kkt-machine stays a contract, and kkt-machine-os an effect.

**Existing installs:** none expected. The merges move code between crates
and change no file, flag, message or command that an install relies on.

## Order at a glance

```
0.1, 0.2
lane A: 1.3 → 1.1 → 1.6 → 1.7    lane B: 1.2 → 1.4 → 1.5 → 1.9    and 1.8
2.1 … 2.7                     two at a time
3.1d → 3.1a → 3.1b → 3.1c-1 → 3.1c-2 → 3.2 → 3.3 → 3.4 → 3.5a → 3.5b → 3.5c
4.1 → 4.2                     after 3.1c-2
1.8 → 5.1a → 5.1b → 5.2a → 5.2b → 5.3a → 5.3b → 5.4a → 5.4b
5.5                           after 1.6, beside 5.1 and 5.2
5.6a → 5.6b                   after 6.3
6.1 (after 3.1b) → 6.2 (after 5.4) → 6.3
7.1a (after phase 3) → 7.1b, 7.2, 7.4, 7.5, 7.6 in parallel; 7.3a → 7.3b → 7.7
8.1 → 8.2
9.1 (kkt-build-b and kkt-build-c in parallel) → 9.2 → 9.3
  → 10.2a → 10.2b, with 10.3 beside them → 10.4 (charts 9 and 10)
```

Three builders can work most of the time: one on phases 1 and 2, one on
phase 3 then 4, and one on phase 5. Phases 6 and 7 follow them. The rule for
the shared constructors (above) holds across all of them.

## The first steps to hand out

Once this plan merges, three steps can start at once, each touching crates
none of the others touches:

- **0.2, the scorecard** (kkt-mt-step1): only `docs/architecture`'s scripts
  and README.
- **1.8, kkt-search:** only kkt-db, kkt-db-sqlite and kkt-store, the only
  crates that name `kkt_db::search`'s text functions today.
- **1.3, kkt-entropy and kkt-entropy-os:** kkt-names, kkt-admission,
  kkt-rules, kkt-web, kkt-server, kkt-mcp, kkt-identity, the programs and
  `tests/common`, none of which 1.8 touches.

They meet only in the workspace `Cargo.toml` and `Cargo.lock`, and in
different rows of CLAUDE.md's crate table and `concerns.json`: a rebase, not
a conflict. After 1.3, lane A goes on with 1.1; lane B (1.2) can start as
soon as 1.3 has merged, since both change the programs and `tests/common`.

## What needs Sam

Each has the default we proceed on, so nothing waits.

1. **CI on `refactor`.** The plan's pull request adds `refactor` to CI's
   branch lists. Default: keep it until `refactor` merges into `next`, then
   remove it in the pull request that merges `refactor` into `next`.
   Accepted by Sam, 2026-11-12.
2. **Breaking the shared crates' public surface.** kkt-pro builds on
   kkt-rules, kkt-store and kkt-server by git, and the refactor removes
   kkt-store and changes these crates' contracts. Default: proceed, since the
   proposal is the decision. Step 8.1 writes one CHANGELOG.md entry for the
   whole refactor, for crates that depend on kkt by git; no step writes its
   own. The outside-crate CI check is kept building. Accepted by Sam,
   2026-11-12.
3. **An unavoidable effect on an existing install.** Default: none is
   expected. A step that finds one stops, posts the question, and leaves the
   rest of the plan going.
4. **The preview program turning on kkt-api-sim's `testing`** (F8).
   `check-crates.sh` rule 3 forbids turning on another package's `testing`
   outside dev-dependencies. Default: step 6.3 lets a dev-only program
   (`[package.metadata.kkt] dev-only = true`) do so, and says so in the
   script's header and in CLAUDE.md. Accepted by Sam, 2026-11-12.
5. **Log output.** F5 changes how kkt-mcp and kkt-web log. Default: the same
   events, targets, levels and fields reach the log file and the terminal.
   Accepted by Sam, 2026-11-12.
6. **How `refactor` merges into `next`.** Default: one pull request, merged
   with a merge commit, no force-push, after a final review of the whole
   diff and a full CI run. Accepted by Sam, 2026-11-12.
7. **macOS in CI.** The macOS Rust checks were off for most of the
   refactor, since GitHub had too little macOS runner capacity, and
   platform-specific code was reviewed by hand meanwhile. They came back
   on `refactor` (#486), then went off again the same day, since GitHub's
   macOS runners are short; "Temporary breaks" lists it. The merge's pull
   request removes `refactor` from CI's branch lists. Accepted by Sam,
   2026-11-12.
8. **The proposed map after the refactor.** Default: step 8.1 keeps the
   proposal as a record of what was planned, with the scorecard at the end,
   and the page's "Current" map shows the code. Accepted by Sam, 2026-11-12.

## Final scorecard

On the code at the end of phase 12, at 1007e1a (#514 merged), against the
proposal with phases 9 to 12 charted (`progress.py`, step 12.6):

- crates: 72 of 72 proposed crates exist; none missing; none to remove;
- items: 794 of 794 proposed items are in their proposed crate;
- arrows: 267 of 267 proposed arrows exist; none missing; none extra;
- rules: no kind check fails; no break of the proposal's rules.

The one temporary break left is CI's, not the code's: macOS is out of CI
(#501), until it comes back or a self-hosted runner takes it.

### At the end of phases 9 to 11

On the code at 1be030c (#502 merged): 61 of 61 crates, 813 of 813 items,
238 of 238 arrows, none extra, no break.

### At the end of phase 8

On the code at step 8.1:

- crates: 60 of 60 proposed crates exist; none missing; none to remove;
- items: 797 of 797 proposed items are in their proposed crate;
- arrows: 222 of 223 proposed arrows exist; 1 missing (kkt-server →
  kkt-api-admin, 5.6a's row); none extra;
- rules: no kind check fails; 1 break of the proposal's rules (kkt-rules'
  tests on kkt-db-sqlite, 5.3b's row).

`decisions.md` ("After the refactor") gives each remaining difference with
its reason.

### Against phase 9's proposal

With phase 9 charted, and kkt-config-file taking the config home folder as a
path (msg 2779), the code at 2715da9 (#480 merged) scores lower, as
expected until phase 9 and kkt-build-c's change land:

- crates: 59 of 59 proposed crates exist; none missing; 1 to remove
  (kkt-settings);
- items: 786 of 795 proposed items are in their proposed crate. Missing:
  kkt-server's `ServerSettings`, kkt-backup's `Settings`, kkt-hosts'
  `DEFAULT_BIND` and `DEFAULT_ALLOW_ANY_BIND`, kkt-logs' two level
  defaults, and the program's `ServeOptions`, `ServeSettings` and
  `server_settings`;
- arrows: 213 of 216 proposed arrows exist; 3 missing (kkt-keymode →
  kkt-backup and → kkt-names, which 9.3 adds, and kkt-server →
  kkt-api-admin, 5.6a's row); 8 extra (the 7 to and from kkt-settings, and
  kkt-config-file → kkt-machine until kkt-build-c's change merges);
- rules: no kind check fails; 1 break of the proposal's rules (kkt-rules'
  tests on kkt-db-sqlite, 5.3b's row).

### Against phases 9 and 10's proposal

With phase 10 charted, with 9.2's `Parts` taking `AdminApi`, with
kkt-service's `Manager::for_this_os` taking plain paths (msg 2829), with
kkt-keymode keeping its own surface, and with the serving code staying in
kashkeshet (msg 2876), the code at 50e6e8e (#488 merged) scores lower
again, as expected until phases 9 and 10 land:

- crates: 59 of 61 proposed crates exist; 2 missing (kkt-operate,
  kkt-agent-commands); 1 to remove (kkt-settings);
- items: 745 of 811 proposed items are in their proposed crate. Missing:
  phase 9's 9 (kkt-server's `ServerSettings`, kkt-hosts' two defaults,
  kkt-logs' two defaults, and kashkeshet's `ServeOptions`,
  `ServeSettings`, `server_settings` and `new_server_run`), and the 57 in
  the two assemblies: their `Parts`, the contracts they declare, and the
  commands that move out of kashkeshet;
- arrows: 210 of 239 proposed arrows exist; 29 missing (the assemblies'
  24, kashkeshet's 2 to them, kkt-keymode → kkt-backup and → kkt-names,
  which 9.3 adds, and kkt-server → kkt-api-admin, which 9.2 makes); 9
  extra (kashkeshet → kkt-api-operator, which moves to kkt-operate,
  kashkeshet → kkt-settings and kkt-keymode → kkt-settings, kkt-settings'
  5, and kkt-server → kkt-rules until 9.2);
- rules: no kind check fails; 1 break of the proposal's rules (kkt-rules'
  tests on kkt-db-sqlite, until kkt-build-b's stand-in replaces it).

With 10.2a (kkt-operate's base) and 10.3 (kkt-agent-commands) merged:

- crates: 61 of 61 proposed crates exist; none missing; none to remove;
- items: 774 of 811 proposed items are in their proposed crate. Every item
  of kkt-agent-commands is in. Missing: what 10.2b moves into kkt-operate
  (setup, service and lifecycle); the 16 `kkt_rules::testing` re-exports
  that #492 removed and `new_server_run`, which 9.3 left in kkt's
  `server_run`, both for 10.4 to chart;
- arrows: 237 of 238 proposed arrows exist (the proposal lost
  kkt-agent-commands → kkt-mcp in 10.3); 1 missing (kkt-operate →
  kkt-backup, which 10.2b adds); none extra;
- rules: no kind check fails; no break of the proposal's rules.

### 12.1 pilot: remote sharing through its scope contract

`kkt-remote-access` now declares `RemoteAccess`: prepare, serve, served and
stop, in project terms, with `Node` and its `testing::Memory` stand-in.
`kkt-remote-access-tailscale` implements it directly. Its private runner
separates process execution (behind `real`) from arguments, parsing,
refusals and messages; the original decision tests use a private fake with
no `real` feature. It replaces kkt-remote-access-os and the program's
`machine/remote_access.rs` adapter. The program injects `Tailscale` directly
into `kkt_operate::OperateParts::remote`.

The rule text gains Sam's third-party exception, including the explicit
exception allowing tool knowledge inside an effect wrapper. The existing
checks already allow any implementation of a contract crate: they do not
count its consumers. Their conditions need no relaxation. Contract kinds
still reject injected behaviour, wiring and `real`; check-crates still
keeps `real` and gated items out of contracts, and requires only programs
to activate another crate's `real`. The scope becomes contract, and the
named implementation stays effect. No file in `principles/` changes.

The map's existing direct-implementation detection is used: the program's
arrow to the constructed implementation is wiring, and the hand-written
injection entry targets the receiving `OperateParts::remote` slot. The
fresh generated map is checked for both before publishing this pilot.

### 12.2: container contract and Docker wrapper

`kkt-container::Container` is the installation boundary. Its operations cover
provisioning, status, existing installs, keys, starting and stopping, backup,
upgrade, lanes and the managed settings questions. `testing::Memory` provides
installations in memory. The contract carries requests and results, plus the
existing config parser and login link contracts. It has no process or file I/O.

`kkt-container-docker::Docker` implements it directly behind `real`. The old
Docker decisions, timings, Compose assets, commands, messages and sidecar status
move into that wrapper, with a private `Runner` and the old fake and native tests.
The program's adapter and `kkt-container-os` go. The program wires `Docker` into
`kkt-operate::OperateParts.container`. No check condition or rule text changes.

### 13.1: container contract and Docker implementation together

The contract and its `testing` stand-in remain outside `real`. The old Docker
wrapper moves into private modules of kkt-container, with Docker re-exported
only behind `real`; its private logic and fake still compile in unit tests
without `real`. The program builds the same Docker directly from kkt-container.

Implementation-only dependencies (the browser contract, config-file, machine,
serde_json and thiserror) are optional and enabled by `real`, with dev-dependencies
for the private logic tests. No consumer turns on `real`, no forwarding adapter
is added, and no command, file, message or test assertion changes.
