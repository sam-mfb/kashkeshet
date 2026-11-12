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

A step that must add a break says so in its pull request and adds a row here
in the same pull request.

## Differences from the proposed map

Some steps end up differing from the proposed map on purpose, for good, not
for a while. They are listed here so Sam can review them in one place. A step
that differs on purpose adds a row in its own pull request; once it merges,
the difference is charted into the proposal (see "Measuring progress"), and
the last column says so.

| Difference | Step | Why | Sam | On the map |
|---|---|---|---|---|
| kkt-lane-files declares a narrow git contract, and the program adapts kkt-lane-files-os' git runner to it, beside `Disk` | 2.6 (#413) | Git's meaning (expected absences, the dirty suffix, error wording) is logic and stays in kkt-lane-files; only running the command is an effect | awaiting Sam | yes |
| One program adapter, an enum over kkt-agent-client's `Remote` and `AdminRemote`, meets kkt-watch's `Server`, in place of two | 1.9 (#420) | Fewer methods (5 in place of 5 + 5), the same arrows | awaiting Sam | yes |
| kkt-service-os reports with its own `Finished`, which the program translates to kkt-service's in `run` and `run_attached` | 2.2 (#427) | So kkt-service-os depends on neither kkt-service nor kkt-config; two adapter methods translate | awaiting Sam | yes |
| kkt-remote-access-os returns its own output and I/O error, which the program maps to the remote-access host contract | 2.3 (#436) | The effect has no dependencies; the adapter preserves all output fields and launch-failure advice | awaiting Sam | yes |
| kkt-client-config-os states its own 0600 private-file mode beside the file operations | 2.4 (#437) | The same fixed value moves with its writer early, so the effect depends on neither client-config nor config | awaiting Sam | not a map change |
| kkt-container-os' command runner takes and returns values of its own, which the program translates to kkt-container's `Command` and `Finished`, and it states its own private file mode | 2.1 (#422) | So kkt-container-os depends on neither kkt-container nor kkt-config; two adapter methods translate instead of forwarding | awaiting Sam | yes |
| kkt, kkt-keymode and kkt-fake-product depend on kkt-format (later kkt-mcp-formatter), kkt-web, kkt-events and kkt-mcp, arrows the proposal does not draw | 3.1d (#424), 3.1a (#429), 3.1b (#431) | The program fills each crate's own settings, so it names their types; the proposal draws those types only from kkt-server | accepted by Sam, 2026-11-11 | yes |
| Each crate takes its tuning values as its own `Settings` (kkt-db-sqlite's `Options`) with a `DEFAULT`, which the program passes, where the proposal shows item 7's constants; a constant only other crates' tests read sits in its crate's `testing` module; the fake product fills each part's settings itself and does not depend on kkt-settings | 3.1a (#429), 3.1b (#431), 3.1c-1 (#445) | Item 7 marked these values "injected"; a settings struct with its default is how a crate takes them, and the program can still pass the default unchanged | awaiting Sam | yes |
| kkt-diagnostics has public `DOCTOR_MAX_RESPONSE_BYTES` and `DOCTOR_PROTOCOL_VERSION`, which the program passes to kkt-diagnostics-os' `Settings`; kkt-agent-client's `Settings` is outside `real` | 3.1c-1 follow-up (#445) | kkt-diagnostics-os chooses no values of its own, and depends on no other crate (2.5); the client's settings are plain data, which the program fills without `real` | awaiting Sam | yes |
| The 0.10.0 migration test stays in the program's tests; only `legacy()`'s facts move into kkt-rules | 5.2b-1 | It builds real SQLite files, and F16 keeps effect crates out of kkt-rules' tests | accepted by Sam, 2026-11-11 | not a map change |
| kkt-diagnostics-os' probe still interprets the server's answers (status meanings, advice text) as it sends them | 2.5 | Splitting it needs the probe's sequence to move into kkt-diagnostics; a candidate follow-up | accepted by Sam, 2026-11-11 | yes |
| Backup logic supplies an immutable Layout (names, copies, metadata, modes); backup-os writes it and asks logic which old names to remove | 2.7 (#440) | Keeps backup decisions in logic while the effect depends only on backup, as the proposal draws | awaiting Sam | yes |
| A package's own `testing` feature may turn on a workspace dependency's `testing` (check-crates rule 3) | 5.2b-1 | kkt-rules' test helpers need kkt-db's in-memory database; `testing` is still reached only from dev-dependencies | decided by Sam, 2026-11-11 | not a map change |
| kkt-config-file-os has five readers and writers, not three: `read` (setup's edit: a missing file is empty), `read_default` (`kkt serve`'s default file: skipped if it cannot be seen), `read_named` (a `--config` file: must be there), `read_instructions` and `write`, with its own `ReadError` and `WriteError` | 3.3 (#445) | Each keeps the behaviour and messages its caller had; the error types are the crate's own, not std's | awaiting Sam | yes |
| kkt-config-file's `parse_file` takes the caller's log-level check, and `Config` carries what it makes of the levels; kkt-container's `ConfigReader` returns its own `ContainerConfig` | 3.3 (#445) | kkt-config-file need not know kkt-logs, and of two bad settings the first by name is still the one named, as before | awaiting Sam | yes |
| `config_home` is a method of kkt-machine's `Env`, with `absolute_xdg_config_home` and `absolute_xdg_data_home`, not a function of kkt-config-file; kkt-config-file depends on kkt-machine (`config_dir` and `config_file` take `Env`); `kkt_admission::access_key_file` and kkt-setup's record paths take the config folder, not `Env`; the program has `NewKey` and `NewAvatar` | 3.4 (#448) | `config_home` is shared by other programs' config (kkt-service's and kkt-client-config's), and the XDG values count only when absolute, so `Env` gives them in one place; a path inside kkt's config folder takes that folder; `NewKey` and `NewAvatar` group two commands' arguments to stay within clippy's limit | awaiting Sam | yes |
| `serve_bind` stays in the program; the service's bind check is kkt-service's `check_bind`, supplied with kkt-service-os's `try_bind` by the program; kkt-keymode keeps four key-refusal messages of its own beside kkt-admission's `key_refusal` | 3.5b (#449) | `serve_bind` gathers the settings from the machine, which kkt-hosts cannot; `check_bind` decides and words the refusal, and trying the address is an effect of service-os (7.3a); kkt-keymode's messages differ from kkt's, and its output stays the same | awaiting Sam | yes |
| kkt-settings has no `SettingsSource`: kkt and kkt-keymode turn kkt-config-file's `Config` and the flags each into kkt-settings' `ServeOptions`, which kkt-settings combines; `Sources`, `Loaded` and `read_config` stay in the program | 3.3 (#445), 3.5c (#450) | `ServeOptions` already is a plain record of one source's optional settings; a trait over it would add a method and two adapters with no new behaviour | awaiting Sam | yes |
| kkt-settings depends on kkt-admission (`AccessKey` in `ServeSettings`, the key paths), kkt-logs (`LogLevel`, `LogPolicy`), kkt-mcp-formatter and kkt-web (the bounds `Limits::check` reads, now public) and kkt-names (`NamePack`) | 3.1b (#431), 3.5b (#449), 3.5c (#450) | Its values hold those types, and `Limits::check` reads the bounds their owners state. None of them depends on kkt-settings, and only the programs do | awaiting Sam | yes |
| Each program keeps `check_settings`: a short function that turns kkt-settings' values into each crate's settings and calls each owner's check, with the mapping of `Limits` to kkt-rules' and kkt-web's settings (about 25 lines) in both kkt and kkt-keymode; `Limits::check` stays one ordered list; kkt-keymode keeps its own `KeyFiles` | 3.5c (#450) | No allowed owner exists for the mapping: kkt-rules and kkt-web may not know kkt-settings, nor kkt-settings them. One ordered list keeps the first bad limit named the same. `KeyFiles` goes with the proposal's kkt-admission-os (7.2) | awaiting Sam | yes |
| kkt-settings words the settings a run records (`RecordedSettings`); kkt-provenance's `RunSettings` holds those lines and depends only on kkt-hash; kkt-service's plan takes its own `ServiceSettings`; kkt-container's `status` and `lane_limit` take their defaults from the program; `DEFAULT_ALLOW_ANY_BIND` is private | 3.5c (#450) | The wording moved unchanged with the golden tests that pin it, and the run record stays byte for byte the same; kkt-settings' defaults are `Limits::DEFAULT`'s, so no separate `DEFAULT_NAME_PACK` | awaiting Sam | yes |
| kkt-logs supplies explicit pure rollover and age-pruning plans, which logs-os performs | 4.1 | Keeps thresholds, rename order and expiry rules in logic while retaining the writer's existing I/O order | awaiting Sam | yes |
| kkt-logs' plans are `should_roll`, `rotation` (with `Rotation`), `prune_due` and `pruning` (with `Pruning` and its `expired`), in place of `rolls_over`, `rolled_name` and `expired`; `LOG_PRUNE_INTERVAL` stays a private constant, not a setting; kkt-logs has its own `SettingError`; kkt-logs-os keeps the program's names `init_logging` and `init_stderr_logging`, not `start` and `start_stderr` | 3.1c-2, 4.1 (#452) | Found when charting 4.1, with no row of their own. A plan names a whole rollover or pruning, so the writer only carries it out; the interval kept the fixed value it had; kkt-logs checks its own settings and may not use kkt-settings' error, as kkt-web, kkt-admission and kkt-hosts; the start functions moved unchanged | awaiting Sam | yes |
| The shared log contract carries borrowed named Field/Value records with Display or Debug formatting, Level::Trace, and testing::Entry | 4.2 | Preserves all five severity levels and structured fields without exposing tracing types or flattening fields into text; Entry retains captured events for assertions | awaiting Sam | yes |
| kkt-api-agent's `Audit::AdminEndpoint` holds the admin key's name as text (`Option<String>`), not kkt-names' `AdminName` | 5.3a | A contract crate holds the already-checked name as a plain value and depends on no kkt crate; kkt-names checks the name where kkt-identity-keymode builds the `Audit` (msg 2476) | awaiting Sam | not a map change |
| kkt-api-agent also holds `LaneSource`, which the proposal left in kkt-rules | 5.3a (#451) | `Lane`, a shared record, holds it, and a contract crate may not hold a record of kkt-rules; found when charting, with no row of its own | awaiting Sam | yes |
| kkt-api-operator also holds `Overview`, `ChannelView` and `LaneHistory`, with `Overview`'s `AgentActivity` and `ActivityCount`, which the proposal's list of its records left out | 5.4a | `Views::overview`, `Views::channel_view` and `Lanes::lane_history` return them, and a contract must own what it returns (msg 2503) | awaiting Sam | yes |
| kkt-api-operator's operations return an `Answer`, a boxed future of their result; since 5.3b the type is kkt-api-agent's, shared by all three kkt-api crates | 5.4a (#455), 5.3b | So the traits work through `dyn`, as kkt-identity's `Identify` does; found when charting, with no row of its own | awaiting Sam | yes |
| Terminal supplies flush_output, message and output_is_terminal beside output and errors | 7.1a | Preserves prompt flushing, each watch message's locked write and flush, and the browser's separate stdout-terminal check | awaiting Sam | yes |
| Terminal::interactive checks stdin, while output_is_terminal checks stdout separately | 7.1b | Preserves setup asking when stdin is a terminal even if stdout is redirected; browser opening still requires both | awaiting Sam | yes |
| Key-file helpers take a caller-supplied key factory; make_default_key reports its created path, write_admin_name takes bound text, and KeyFiles adds read, read_optional and create_dir_all with ReadError retaining its path and I/O cause | 7.2 | Keeps admission-os dependent only on admission; the program preserves its info-event target and admin-name wording, and the two KeyTexts adapters translate native read errors into each program's existing refusal messages | awaiting Sam | yes |
| kkt-rules' tests use kkt-db-sqlite's in-memory database, an effect crate's, not kkt-db's `MemoryPlace` | 5.3b | Speed: the 200 tests take under 5 seconds on SQLite in memory and over 10 minutes on kkt-db's `MemoryPlace`, which copies the whole database on every change. kkt-db's conformance checks keep both in step, and kkt-rules' `testing::memory` still serves other crates (msg 2498) | awaiting Sam | noted on the arrow the proposal removes: its rule keeps effect crates out of a logic crate's tests, so the scorecard counts this as a break |
| Two combined traits: kkt-api-operator's `OperatorApi` (`Api` + `Views` + `Changes` + `Lanes` + `Server`, with `in_swarm_operator`) and kkt-api-admin's `AdminApi` (`OperatorApi` + `Admin`, with `in_swarm_admin`); kkt-api-admin depends on kkt-api-operator | 5.4b | An endpoint that moves into the caller's swarm needs every operation it uses in that swarm, which `Api::in_swarm` alone cannot give; `Rules` implements both, so no adapter (msg 2562) | awaiting Sam | yes |
| kkt-mcp-formatter no longer depends on kkt-rules (only its tests do, for kkt-rules' default settings) | 5.5 | The proposal kept the arrow from the old map; with the wording in kkt-rules and the records in the kkt-api crates, the formatter's library uses nothing of kkt-rules (msg 2580) | awaiting Sam | yes |
| kkt-web's `Sessions<T>` is generic over what a session holds (bound by its `Held`), `Gate::SignedIn` (kkt-server's `Principal`, the preview's `()`); `Gate::admits` takes kkt-web's `Session<T>` (absent, ended or live) and `Gate::sign_in` returns `SignIn<T>` (admitted, or refused with a reason) in place of a session ID; `Sessions::new` takes an `Access` (open or sign-in), from which the page knows whether sign-in is required; the lifetime is `session_lifetime` in kkt-web's `Settings`, so kkt-server's `AppConfig` loses `sessions` and the program's `session_settings` goes (its `web_settings` fills the lifetime); kkt-web's `testing::Door` holds a gate and its sessions for the page scenarios | 6.1 | The page keeps the sessions, so the gate is asked about what a session holds, not its ID; form tokens need to know whether sign-in is required, so the sessions know it from construction and `Gate` drops `sign_in_required`; one settings value per crate | awaiting Sam | yes |
| kkt-admission's `WebAdmission` decides over its own `WebSession` (absent, ended or live with a `Principal`), and `WebAdmission::sign_in` returns a `Decision`; its `SignIn` and `Settings` go | 6.1 | kkt-admission keeps the access decision and its refusal texts ("sign in first", "the session has ended; sign in again") without knowing kkt-web's sessions | awaiting Sam | yes |
| kkt-api-operator's `Views` has `subscribe`, which gives the operator's `Notices` (an async `next` of a `Notice`, or `None` at the end), with its own notice records (`NewAgent`, `ChannelCreated`, `Joined`, `Left`, `ChannelHidden`, `AgentHidden`, `ChannelMembers`, and `Notice::Missed` with no count; a message is kkt-api-agent's `Message`); it checks `Read`. `Rules` adapts kkt-events' subscription to it, and kkt-web serializes the records for its live feed; kkt-web's `LiveEvent`, `live_event`, `NewAgent`, `NewChannel`, `Joined`, `Left` and `testing::ChannelHidden`, which the proposal keeps, go | 6.2 | The proposal has kkt-web draw live updates from kkt-api-operator's subscribe but names no types for it, and a contract may not name kkt-events; the records carry exactly the fields the page sends, in the same order, so the feed is unchanged | awaiting Sam | yes |
| kkt-web's `Gate` has no `Viewer` type of its own choosing: it admits a `Viewer` record of kkt-web (a `SwarmId` and an `Operator`), and kkt-server's `WebViewer` goes; the page takes the application as one `Arc<dyn OperatorApi>` and asks `in_swarm_operator` for each request's viewer | 6.2 | The page must call the operations in the viewer's swarm as the viewer's operator, so it must read both; the proposal left `Viewer` opaque while removing `Sources`, which read it | awaiting Sam | yes |
| kkt-web's `Settings` holds `page: PageLimits` in place of `page_messages` and `member_excerpt_chars`; the programs fill it, and kkt-server passes it in `Settings` as before | 6.2 | One settings value per crate; `PageLimits` is the part only the page applies | awaiting Sam | yes |
| The preview keeps its simulated world in records of its own (kkt-api-sim's private `records`, the old contract's records) and translates them to the kkt-api crates' records in its `api` module; it refuses every operation the page never asks; it makes a world for each swarm asked for, the first time | 6.2, kept in 6.3 | Changes the preview's internals as little as possible; a world per swarm lets kkt-web's own test see that each request reads its viewer's swarm | awaiting Sam | yes |
| kkt-web's `testing` scenarios take an `App` (an `OperatorApi` and the operator the scenario checks as) in place of a `Source` | 6.2 | The scenarios read the application to check what the page did, and every operator operation takes the operator | awaiting Sam | yes |
| kkt-rules' `start` takes `NewServerRun` and a `Startup` (the rules' `Deps` without the run, which `start` fills in) and returns the `Rules`; `Rules::run_id` gives the run's ID, for the program's log line; kkt-server's `new_server_run` turns kkt-provenance's `ServerRun` into a `NewServerRun`; kkt-server's `Parts` holds the `Rules`, the clock and entropy the page's sessions use, `identify`, `log` and the formatter, and `Parts::new` takes those, in place of the database, wording, events settings and run ID; `AppConfig` loses `rules` (the settings go into `Startup`); the program makes the events bus and stops it with the server's stop; kkt-server's `Deps` wiring leaves assembly.json, and each program's wiring goes to `Startup` and `Parts::new (rules)` | 5.6a | The proposal names `start` and `NewServerRun` but not how `start` gets the rest of `Deps`; one `Startup` struct keeps `Deps` as it is for tests that build `Rules` directly; kkt-rules may not know kkt-provenance, and kkt-server already does, so it maps the run; the program now builds the rules, so it owns the bus the rules hear on, and kkt-server depends on kkt-events only in its tests; kkt-keymode depends on kkt-api-agent for the swarm it names in `Startup` (`SwarmId::main`) | awaiting Sam | yes |
| kkt-server's `Parts` holds the concrete `Rules`, so kkt-server does not name kkt-api-admin's traits; the proposal's arrow kkt-server → kkt-api-admin is missing | 5.6a (#476) | `Parts` passes the one `Rules` to each surface, which takes it as its own contract (`Api`, `OperatorApi`, `AdminApi`); naming the traits in `Parts` would add a type parameter or a boxed value without changing what any surface can reach | awaiting Sam | not charted: the scorecard counts it as one missing arrow, recorded for 8.1 |
| The store goes private except through kkt-rules' `testing`: the program's tests that pin what a 0.10.0 database answers (`tests/migrations/v0_10_0.rs`, every `Reader` question) and that write rows into a database file (`tests/database_files.rs`) reach `Reader`, `Writer`, the records and filters they take, and `testing::read` and `testing::write`; `testing::database` gives a `Db` for a program's own `start` (the fake product's tests) and the operator's `CommandLine`; `testing::memory` takes a `testing::Memory` (`Startup` without the database, with defaults from `Memory::new`); `Reader::recent_messages`, which no rule asks any more, is behind `testing` | 5.6b | The 0.10.0 fixture must answer every pinned read (CLAUDE.md), and those reads are the store's questions, which the kkt-api operations do not ask one by one; the file tests write rows to check how a file is kept, not what a rule does. Behind `testing`, a released build reaches none of it | awaiting Sam | after it merges |
| Credential readers take a CredentialCheck over named CredentialFileFacts and a warning callback; CredentialFile takes a cleanup-warning callback; PrivateMode and CredentialFileFix carry the checked mode, advice and warning, and FileLock exposes LockRefused | 7.4 | Keeps agent-client independent of setup and terminal effects; setup owns the rule over caller-quoted paths, both owner checks and warning-before-read ordering stay intact, and cleanup warnings keep their original log target | awaiting Sam | yes |
| kkt-web-preview is a program only, a binary with no library (kind composition, not logic): its arrows are [dependencies] of the binary, not dev arrows of an example, and its `SettingError` is the binary's own, not a public item; the preview's tests (the page scenarios over `Preview`, and tests of its world, feed and gate) are kkt-api-sim's unit tests, beside the stand-in they test; the program's own tests are of its flags | 6.3 | Decision 39 leaves kkt-web-preview only the program (A7: it counts as a program), and check-crates.sh lets a dev-only program, not an example, turn on `testing`; tests of a stand-in sit beside it | awaiting Sam | yes |
| kkt-server has `page`, which builds an `App` of the web page alone over any `OperatorApi` and gate; kkt-web-preview serves it through kkt-server-listen, so it depends on kkt-server and kkt-server-listen, arrows the proposal does not draw | 6.3 | So the preview program serves the page without naming axum, which stays in kkt-web, kkt-mcp, kkt-server and kkt-server-listen, and without binding a socket in its own code | awaiting Sam | yes |
| kkt-web-preview logs through kkt-logs-os (`init_stderr_logging`, `TracingLog`, with kkt-logs' `LogLevel` and kkt-log's `Level`) and reads its key file through kkt-admission-os (`KeyFiles::read`), arrows the proposal does not draw; it drops `tracing` and `tracing-subscriber` | 6.3 | So the program does no machine work in its own code (check-programs.py) | awaiting Sam | yes |
| kkt-api-sim has `PreviewSettings` (the feed's steps per minute) and its own `SettingError`; the page size is kkt-web-preview's flag, checked by the program (its `SettingError`) into kkt-web's `PageLimits`; the preview's bind address, seed and session lifetime are the program's (the lifetime is kkt-web's default); kkt-web's `SettingError`, which only the preview used, goes; kkt-api-sim's `PREVIEW_SWARM`, `preview_viewer` and `Preview::swarms` are public, for kkt-web's viewing test | 6.3 | Each crate declares the settings it takes and its check's error (decision 42); the simulation never used the page size | awaiting Sam | yes |
| kkt-api-sim depends on kkt-shutdown: `Preview::new` takes the stop that ends its live notices and `Preview::run` the one that ends its feed; kkt-web no longer depends on kkt-events, which only the preview used since 6.2 | 6.3 | The stop is the caller's, as it was in kkt-web; the page hears notices through kkt-api-operator's `Notices` (6.2) | awaiting Sam | yes |
| kkt-mcp's tests do not use kkt-api-sim | 6.3 | They already use only stand-ins (the fixed clock, counting entropy, the rules over an in-memory database) and need nothing the simulation gives; the arrow can come when a test does | awaiting Sam | yes |
| kkt-fake-product depends on kkt-agent-client with real as a normal dependency, not dev | 7.5 | check-crates allows real in dev-dependencies only for examples; the fake product is a dev-only binary package; only its integration tests use this dependency (msg 2604) | awaiting Sam | yes |
| Http returns an opaque HttpReply with status, header and text/byte-stream methods; SqlitePlace::create reports CreateError::Exists or Open | 7.5 | Keeps HTTP library types inside agent-client and preserves streaming/form behavior; the program keeps its existing-file and failed-open wording | awaiting Sam | yes |
| Setup's client/admin connection and endpoint decisions use consumer-owned `ConnectionSource` and `ServeConnection<S>` records; recorded selection takes a path-resolution callback, and `recorded_setup` takes lazy unfinished/unrecorded readers | 7.3a | Preserves the first read/error and avoids setup depending on program settings, diagnostics or native providers | awaiting Sam | yes |
| Setup-os adds `path_dirs`, `absolute`, `canonicalize`, `write_unfinished` and `remove_unfinished`; container Host and native key files expose `is_file` | 7.3a | PATH is captured once for all client checks; path I/O stays native; note writes keep their original rename and missing-file semantics; install presence distinguishes files from directories | awaiting Sam | yes |
| `docker_env` returns settings text or the original read refusal, rather than an absent value for a missing file | 7.3a | Upgrade previously refused a missing .env; Host now performs that same read and the caller keeps its version-line refusal | awaiting Sam | yes (`docker_env`'s description) |
| Setup returns plain `DoctorCheck` and lane-import `Outcome` values, adapted by the program from/to the diagnostics and operator records; service checks receive installed/running facts | 7.3b | Owns each check, fix and import report without setup depending on diagnostics, service or operator implementation crates; program adapters copy facts only | awaiting Sam | yes |
| Setup adds status-formatting, backup-folder, uninstall-record, key-naming and lane-limit functions; setup-os removes records, SqlitePlace checks file presence and KeyFiles::try_exists retains probe errors | 7.3b | Moves the remaining program-work decisions and file probes to their owners while preserving read order, messages and missing-file semantics, including path calls the original scanner missed | awaiting Sam | yes: the proposal's `server_key_file_name` is `client_key`, with `staged_client_key` |
| kkt's `local_web_key` and `local_operator_key` stay in the program, not kkt-setup: they read the record, the config folder and the configured key in kkt-setup's order, and kkt-setup's `operator_key` and `configured_key` decide | 7.3b (#473) | Found when charting, with no row of its own: the decision is already kkt-setup's, and what is left is the reads | awaiting Sam | yes |
| kkt-server's tests use kkt-api-sim's `Preview`, `PreviewGate` and `PreviewSettings`, and kkt-web's tests stop the simulated feed with kkt-shutdown's `Shutdown` (test arrows the proposal does not draw) | 6.3 (#470) | Found when charting, with no row of its own: the page tests over the simulated application moved with it, and a manual stop is one of the stand-ins the proposal gives kkt-web's tests | awaiting Sam | yes |
| Admission's `server_keys::bound_to` and `server_keys::bound_in` are free functions over names' `AdminName`, and names drops its hash dependency; `server_keys::admin_name_line` takes the Docker-copy fact and a lazy name-text reader, and `server_keys::admin_name_written` formats the native write result | 7.7 | Binding belongs to admission while name validation stays in names; cross-crate inherent methods are not possible, and the callback preserves the host-copy no-read rule without an admission-to-container/native dependency | awaiting Sam | after it merges |


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
`config_home` stays a method of `Env` (see the differences). Depends on 3.3.

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
2. **Breaking the shared crates' public surface.** kkt-pro builds on
   kkt-rules, kkt-store and kkt-server by git, and the refactor removes
   kkt-store and changes these crates' contracts. Default: proceed, since the
   proposal is the decision. Step 8.1 writes one CHANGELOG.md entry for the
   whole refactor, for crates that depend on kkt by git; no step writes its
   own. The outside-crate CI check is kept building.
3. **An unavoidable effect on an existing install.** Default: none is
   expected. A step that finds one stops, posts the question, and leaves the
   rest of the plan going.
4. **The preview program turning on kkt-api-sim's `testing`** (F8).
   `check-crates.sh` rule 3 forbids turning on another package's `testing`
   outside dev-dependencies. Default: step 6.3 lets a dev-only program
   (`[package.metadata.kkt] dev-only = true`) do so, and says so in the
   script's header and in CLAUDE.md.
5. **Log output.** F5 changes how kkt-mcp and kkt-web log. Default: the same
   events, targets, levels and fields reach the log file and the terminal.
6. **How `refactor` merges into `next`.** Default: one pull request, merged
   with a merge commit, no force-push, after a final review of the whole
   diff and a full CI run.
7. **macOS in CI.** The macOS Rust checks are off for now, since GitHub has
   too little macOS runner capacity (`ci.yml`'s matrix says so). Default:
   review platform-specific code by hand meanwhile (the reviewer flags any
   `cfg(target_os = "macos")` code a pull request touches), and turn the job
   back on as soon as Sam says capacity allows, at the latest before
   `refactor` merges into `next`.
8. **The proposed map after the refactor.** Default: step 8.1 keeps the
   proposal as a record of what was planned, with the scorecard at the end,
   and the page's "Current" map shows the code.
