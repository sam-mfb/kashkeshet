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

Kinds and features: every crate is one kind, as concerns.json says:
contract, logic, effect, composition or test support. Code that touches
the machine (files, network, processes, the clock, randomness, signals) is
in an effect crate of its own, and only programs depend on an effect
crate, in code or in tests. A contract with one implementation is a
contract crate and a crate named for the implementation that implements it
directly (kkt-bare-service and kkt-bare-service-systemd), or the consumer declares
the contract it needs and the program adapts the effect crate to it
(kkt-llm-agent-config's `ClientCommand`, over kkt-llm-agent-command); logic and
the effect that carries out its plans are a logic crate and its `-os`
crate. One exception: kkt-containerized-service holds its
`ServiceContainer` contract beside `Containerized`, its one
implementation, because that implementation is logic over the injected
`Projects` and touches no machine, so a crate of its own would give its
users nothing. No crate has a `real` feature. Tests SHOULD NOT depend on an
effect crate the program they test does not: a test that must is a
documented exception, listed with its reason in
`.github/scripts/effect-exceptions.json` (today, the fake product's tests,
which drive it over HTTP with kkt-agent-client-mcp). Stand-ins and other test
support are behind a `testing` feature, which other crates turn on only in
their dev-dependencies (RUST.md 6); a crate's own `testing` may turn on
its dependencies' `testing`, and a dev-only program (see "Dev-only
packages") may turn it on in its dependencies. A fake used only inside one crate stays
in that crate's unit tests. CI builds each crate on its own and checks these
rules (`.github/scripts/check-crates.sh`). A crate with no feature listed
has none.

Platform crates: a crate that knows nothing of kkt and would make sense in
any program (time, randomness, logs, files, HTTP serving, the terminal,
the browser, remote access, git) is in the map's `platform` layer
(`docs/architecture/concerns.json`), and depends in code only on other
platform crates. A crate that needs kkt's own words,
such as handles or the setup record, is not a platform crate. A platform
crate says nothing of kkt either: kkt's names, settings, commands and
messages are its caller's, passed in as arguments, or worded by the caller
from the values it returns.

| Crate | Owns | Must not | Features |
|---|---|---|---|
| `kkt-rules` | Every permission decision and every change, for agents and the human; starting the rules for a server run (`start`, with `Startup` and `NewServerRun`), which records the run first; the core's data, in its storage module (`src/store/`): its questions (`Reader`) and changes (`Writer`) in kkt-db's terms, and the core's tables as kkt-db migrations (`src/store/schema.rs`); the bus its notices go on (`src/events.rs`), built by `start` from the `event_buffer` setting and the shutdown signal: new messages, agents, channels and members, and hidden agents and channels, each notice heard only in its swarm, with a buffer per swarm kept only while the swarm has a subscriber | Reach the database except through the storage module's `Reader` and `Writer`; format output; contain SQL or know a particular database (it is given a `kkt_db::Backend`); let a tokio type out of its events module | `testing`: `testing::swarms_stay_apart_in` (the check every store must pass, on any backend); `testing::scenarios::run_all` (the rules' scenarios whose answers rest on how a database answers and that no end-to-end test asserts on the real backend, run over the backend given: the program's `tests/rules_on_sqlite.rs` runs them on kkt-db-sqlite); `testing::memory` (the rules over an in-memory database, after a test run, for other crates' tests, which set up their data through the kkt-api crates), `testing::database` (the database alone, for a program's own `start`) and `fail_next_write`; `testing::rules_on` (the rules over a database the test opened itself); and `testing::every_store_read`, every store read asked on a database with the `Probes` given, as text, for the program's test that an older release's database answers as it did. No store record, `Reader` or `Writer` can be named outside kkt-rules |
| `kkt-api-agent` | What any registered agent may do: `Api` and its areas (`Agents`, `Channels`, `Messages`, `Memory`, `Lanes`), which `kkt-rules` implements, with `Limits`; and the records every level shares: agents, channels, messages, memory, lanes, the queries, who acts (`Actor`, `Agent`, `Operator`, `Audit`, `Permissions`) and the errors (`RuleError`, `Code`) | | none |
| `kkt-api-operator` | What the human who runs the server may do, by area (`Views`, `Changes`, `Lanes`, `Server`), with the records only they use, and `OperatorApi`, all of it in one swarm; the notices the operator hears as things happen (`Notice`, and `Notices`, which `Views::subscribe` gives); `kkt-rules` implements it | Name a logic crate | none |
| `kkt-api-admin` | What /mcp-admin's administrator does: the admin reads (`Admin`), creating a swarm (`Swarms`), `AdminChannel`, and `AdminApi`, every operation /mcp-admin uses in one swarm; `kkt-rules` implements it | | none |
| `kkt-db` | The database contract, for any database: values; declaring tables, keys, references, indexes and constraints; the questions and changes a store may make (a narrow language, not SQL); transactions (a read sees one state, a write runs as if alone); search: what a search asks for (`Search`, `Excerpt`) and finds (`Hit`), with kkt-search's words, matching and excerpts applied here for every backend (`Connection`), so a backend is handed the words it indexes (`Words`, `Reindex`) and the terms it matches (`Terms`), which only kkt-db can make, and turns stored text into words only through `Indexer`, when it builds an index over rows already there; migrations as additions, recorded per named set; `Legacy`, a database from before migrations were tracked, in no backend's terms; the `Backend`, `BackendConnection` and `Evolve` traits a backend implements; `Connection`, what a store asks and changes through; and `BackendExt`, the typed read and write a store calls on any backend | Know a product's tables; name a database or use a database library, except in its `testing` stand-in `FastPlace` | `testing`: `MemoryPlace`, a database in memory that keeps the contract, plainly and slowly; `FastPlace`, a fast stand-in for stores' tests, built on SQLite in memory with a translation of its own (it shares nothing with kkt-db-sqlite); `conformance`, the checks every backend must pass, over a `Place` that gives it fresh databases; `FaultyBackend`, a backend that wraps another and fails a change on cue |
| `kkt-db-sqlite` | kkt-db over SQLite, the only crate that knows SQLite outside tests: SQL written from kkt-db's operations, one writer behind a lock and read-only readers, constraints as triggers, search indexes as FTS5 over the words kkt-db hands it, migrations recorded in `kkt_db_versions`, adopting a database from before kkt-db as the caller's kkt-db `Legacy` describes, reading SQLite's `user_version` itself (`SqlitePlace::adopting`), `copy_database` | Know a product's tables; let a rusqlite type out of the crate | `testing`: `MemoryPlace`, a SQLite database in memory |
| `kkt-search` | Search as text: the words of a text (FTS5 `unicode61`'s rule, as a table of our own), a search's terms, whether a text matches them, and an excerpt around the matches, drawn with the marks and window it is given | Know a database | none |
| `kkt-toml` | TOML text, in kkt's terms: `parse` (a text's tree of values, `Table` and `Value`), `read` and `write` (a typed record from text and back), and `Document`, a text open for editing that keeps its comments and layout (`find`, `table_mut` with `IfMissing`, `list_mut`, `TableMut`, `ListMut`, `Found`, `EditError`); its own errors, which give the parser's reason and the place in the text (`ParseError`, `Place`, `WriteError`). The only crate that uses `toml` and `toml_edit`. Pure logic: its users depend on it directly | Let a `toml` or `toml_edit` type into its public items; know whose file it reads | none |
| `kkt-mcp` | The agent and admin MCP tools; what the two share; the endpoints' HTTP transport and their paths (`MCP_PATH`, `MCP_ADMIN_PATH`); their text; their results turned into text or JSON, within the response size cap (`result_format`: `Formatter`, `Format`, its `Settings`, bounds (`Problem`, `Settings::problems`, `MIN_RESPONSE_BYTES`) and run-record lines (`Settings::record_lines`)) | Touch the database except through the rules; hold tool or parameter text in Rust; decide who a caller is except through `kkt_identity::Identify` | none |
| `kkt-identity` | Who a caller is, as a contract: `Identify`, which turns an admitted caller's `Principal` into (swarm, actor) or a refusal, looking an agent's token up through `Tokens` (`ApiTokens` over the agent operations); the one place that decides who someone is and what permissions they get | Touch HTTP: each surface reads the principal off its own requests | `testing`: `FixedIdentify`, a stand-in `Identify` |
| `kkt-identity-keymode` | Key mode's identity: `KeyMode`, which implements `Identify` from the key that let a caller in and the avatar's token, in the one swarm | Touch HTTP | none |
| `kkt-web` | The human's pages, over kkt-api-operator's `OperatorApi` in the viewer's swarm, rendering the kkt-api crates' records; its own contract is only `Gate` (`src/gate.rs`), which says who is viewing each request (a `Viewer`: a swarm and an operator) and whom a key signs in; the limits only the page applies (`PageLimits`); web sessions (`Sessions`: IDs, hashing, expiry, sign-out and form tokens, holding what the gate said at sign-in), with how long they last in its `Settings`, and its settings' bounds (`Problem`, `Settings::problems`) and run-record lines (`Settings::record_lines`) | Depend on the rules, the store, admission or identity: `kkt-server` adapts them to its gate | `testing`: the page scenarios every implementation of `OperatorApi` must pass (`kkt_web::testing`, over its `App`, with `Door`, a gate and its sessions); turns on `tower`, tokio's `time` and kkt-log's `testing` |
| `kkt-admission` | Whether an HTTP caller is let in at all, and as which `Principal`: no key, which of the server's keys, or (for another program's `Identify` to judge; kkt never sets it) any bearer token; for the web page, whether sign-in is needed and whom a key admits (`WebAdmission`; the sessions are kkt-web's); the server's admission from its keys (`server_keys::check`, over the texts `KeyTexts` gives, as kkt-keys' rules say) and what to say when a key is refused (`key_refusal`) | Know about agent tokens or channel access; decide what a valid key is or where key files go | none |
| `kkt-keys` | The server's keys, as a contract: what a valid key is (`check_access_key`, `SettingError`) and what a new one is drawn with (`NEW_KEY_ALPHABET`, `NEW_KEY_CHARS`); which access key a server uses (`AccessKey`); a key file's text (`KeyText`); the key files' names and mode, where the access key file goes by default (`access_key_file`, in the config folder it is given), and where the web key, the admin key and the admin name's file go beside it (`web_key_path`, `admin_key_path`, `admin_name_path`), and the names of a Docker setup's host key copies (`WEB_KEY_COPY_FILE_NAME`, `ADMIN_KEY_COPY_FILE_NAME`), kept together in one module; the exit code of `kkt key show --admin` when there is no admin key (`NO_ADMIN_KEY_EXIT`); binding names to admin keys, and what the name file beside a key says of it (`bound_to`, `bound_in`, `read_admin_name`, `AdminNameFile`), and what is wrong with it (`NameFileError`, `AdminNameError`); and `KeyFiles`, reading and writing private key files and telling their permission bits (`mode`) (the server's, and the client key copies setup writes), whose provided methods read the server's selected keys (`SelectedKeys`, `ReadError`) and write a new or default key, and why a key file could not be written (`KeyFileError`, with `KeyFileAction`). kkt-keys-os implements `KeyFiles` | Decide who a key lets in; draw randomness; word each program's key refusals; touch the key files | `testing`: `testing::MIN_ACCESS_KEY_BYTES`, the shortest access key, and `testing::Memory`, key files in memory, for other crates' tests |
| `kkt-keys-os` | Private key files on disk: `OsKeyFiles` implements kkt-keys' `KeyFiles` with the operating system's files, as kkt-keys names and places them: reading them, writing a key file only its owner can read, new or replaced when forced, the admin name file, staged-key moves and the file tests | Decide what a valid key is, who a key lets in, or where key files go; draw randomness | none |
| `kkt-hosts` | Which host names the server answers to; whether the server may listen on an address (`check_bind`); the default address and whether any address may be used (`DEFAULT_BIND`, `DEFAULT_ALLOW_ANY_BIND`) | | none |
| `kkt-install-commands` | The commands that install and look after kkt on this machine: `kkt setup` and `kkt switch`, the install's lifecycle (`lifecycle`: status, upgrade, uninstall and backup, writing backups through kkt-backup's `Backups`, with the lines they print: the status lines, the uninstall record and line, the upgrade error); setup's own steps (`setup::steps`: the client key's name, staged and kept per server, and the line said while setup waits for its server), `kkt service` and `kkt doctor`; reading the setup record and what is installed here (`record`, and `record::InstallReads`, whose reads the program adapts to kkt-operator-commands' `InstalledHere`), and kkt-deployment's facts read from them (`deployment`, among them the key facts `server_keys`, `found_server_keys` and `unfinished_setup`), and kkt-backup's `ServerHere` (`lifecycle`); kkt-diagnostics' facts for the doctor and for setup's check of its server, read from them (`doctor`: which setup the doctor goes by, and how the server the clients use is checked); the local key finders setup and the service use (`keys`, over kkt-deployment's finders); `ImportLanes`, the lane check and import setup asks for, which the program adapts; the Docker setup's key copies, from kkt-deployment's `docker_key_copies`, and the config file's path under the container's config home, passed to the container; with every effect and lazy serving question supplied through `InstallParts` | Choose native implementations; depend on another assembly or an effect crate; parse command-line flags | none |
| `kkt-operator-commands` | The operator's commands on a server: `kkt key`, `kkt agent` and `kkt lanes`, and the lane check and import setup asks for (`lanes::check_lanes_for_setup`, `lanes::import_lanes_for_setup`); the server whose lanes they act on, as a contract (`lanes::LaneServer`, with `LaneImport`), which server that is for a kkt-deployment `Deployment` (`lanes::lanes_on`, `LanesOn`), and its two implementations: on the local database (`lanes::LocalLanes`), and kkt in a container (`lanes::ContainerLanes`), over what the lane commands need of kkt there, which this crate declares (`lanes::ServerInContainer`: run kkt's program in it with its arguments, `RunArg`, and the files it carries, `Carried`; whether kkt is installed; its config file and the lane limit it sets) and the program adapts; the program picking the server for `kkt lanes` (`lanes::LaneServers`) and for setup; the report of an import; `kkt key create`'s own check that a name is a valid admin name, made after kkt-deployment's `create_checks`, and the name file it writes beside a new admin key; finding the key files they use (`keys`, over kkt-deployment's finders); what is installed here, as a contract it declares in kkt-deployment's terms (`InstalledHere`: the recorded server's keys, an unfinished setup, a server found with no record), which the program adapts kkt-install-commands' reads to, with every other effect supplied through `OperatorParts`; the lane helpers, setup's lane steps among them, take only the parts they use (`LaneParts`) | Choose native implementations; depend on another assembly or an effect crate; parse command-line flags | none |
| `kkt-server` | Assembling the server from its parts, around the `Arc<dyn AdminApi>` the program gives it (`Parts`); the settings of what it builds (`ServerSettings`, with an opaque `CheckedSettings` snapshot consumed by `Parts::new`), with their problems and run-record lines for the program to order (`SettingError`); the server's adapter to the web page's gate (public, for a product that builds its own page: `AdmissionGate` asks kkt-admission and `Identify`); building the page's `Sessions` and handing it the rules as `OperatorApi` when it mounts the page; the web page alone over any `OperatorApi` and gate (`page`), for a program that serves only the page | Name other crates' settings, or order a run's checks and record; make decisions: it only wires parts together. It does not choose who a caller is: the program passes `Identify` in through `Parts` and the admission modes of `/mcp`, the web page and `/mcp-admin` through `AppConfig` (`/mcp-admin`'s is an `AdminAdmission`, which has no open mode) | none |
| `kkt-files` | General file access on this machine, as plain `Files` methods: reading and writing whole files, a folder's file names, making folders, renaming and removing, whether a path exists, creating a private file and restricting one to its owner, absolute and canonical paths; a file that may be missing (`read_if_present`), a write by rename (`write_by_rename`), a removal where missing is fine (`remove_if_present`) and `is_file`. Each program adapts it to the contract the crate that needs files declares (kkt-lane-files' and kkt-config-file's `Disk`, kkt-llm-agent-config's `Host`) | Know whose files these are or what is in them; depend on another kkt crate | none |
| `kkt-listen` | Serving HTTP: binding an address (`listen`) and answering on it with the router it is handed until told to stop (`Listening`); trying whether an address could be listened on, letting it go at once (`try_bind`, for kkt-deployment's `check_bind`) | Know how the router it serves was built, or depend on the crates that build it | none |
| `kkt-config-file` | The config file's format: `parse_file`, which turns its TOML into `Config`, what the file says in its own terms (log levels as the caller's check reads them); `with_allowed_host`, which adds a host to its text; `ConfigError`; where the config file is: `config_dir` and `config_file`, under a supplied config-home folder, and `config_file_in_home`, its path relative to one; reading and writing the config and instructions files whole (`files`: `read`, `read_default`, `read_named`, `read_instructions`, `write`, with `ReadError` and `WriteError`) over the `Disk` it declares (the program adapts kkt-files to it) | Touch files except through its `Disk`; know kkt-log-tracing or another crate's settings: each program turns a `Config` into its own settings | none |
| `kkt-provenance` | What produced a server run, and the build script that records the git commit; the run's settings come as lines of a name and a value (`RunSettings`), which the program words | Store the run record: kkt-rules' `start` does; know another crate's settings type | none |
| `kkt-clock` | Time: the `Clock` contract, kkt's `Time` and `Span` values, and time formatting. The only crate that uses chrono | Let a chrono type into its public surface; read real time | `testing`: `FixedClock` |
| `kkt-clock-system` | The real wall clock and its sleep, implementing the shared `Clock` contract | Define time values or formatting | none |
| `kkt-names` | Handles, IDs, the name packs, and the operator's and admins' names; the checks of a proposed handle, a lane's name, a memory entry's name and an admin name, and which rule a name breaks (`NameError`, with `Named` and `Rule`), worded for whoever gave it | Draw randomness except through `Entropy` | none |
| `kkt-entropy` | Randomness, as a contract: `Entropy`, with `random_chars`, which draws from the alphabet its caller gives | | `testing`: `SeqEntropy`, `FirstOfLongListsEntropy`, `SeededEntropy` |
| `kkt-entropy-os` | Randomness from the operating system | | none |
| `kkt-machine` | Facts about the machine: `Env` (the home folder, the XDG folders, counted only when absolute, and `config_home`, the base of per-user config that other programs' config shares too, and `data_home`, the base of per-user data under the XDG base directories, and any other folders the program asks for by the variable that names them, `Env::named`) and `Platform`, as plain values; `Machine`, the contract for reading them, this process's ID, this program's path, `$PATH`'s folders and the folder the process works in. The crates that place files take these facts and build their own paths | Read the environment; hold any of kkt's paths | `testing`: `FixedMachine`, a stand-in `Machine` with the facts it is given |
| `kkt-machine-os` | The machine's facts as this process sees them: `OsMachine` implements kkt-machine's `Machine`, reading the environment (the home and XDG folders, and the variables the program names), naming the platform this program was built for, and giving this process's ID, this program's path (`program_path`), `$PATH`'s folders (`path_dirs`) and the folder it works in (`current_dir`). The program asks once at startup and passes its `Env` and `Platform` down | | none |
| `kkt-shutdown` | The shared stop signal (`Shutdown`), a concrete in-memory value; `Signals`, the contract for a source of stop signals, which stops a `Shutdown` and says which `Signal` came | Let a tokio-util type out of the crate | `testing`: `OnCue`, a stand-in `Signals` whose signal comes when the test says |
| `kkt-shutdown-os` | The operating system's stop signals: `StopSignals` implements kkt-shutdown's `Signals`, listening when it is made (`listen`) and stopping the shared `Shutdown` value when one comes | Let a tokio signal type out of the crate | none |
| `kkt-log` | The shared structured log contract: levels and their names, targets, messages and named values | Know a logging library or perform I/O | `testing`: `CapturedLog`, with the `Entry` records it keeps |
| `kkt-terminal` | The terminal's output, input and terminal detection, the closed-output-pipe default, and a link shown with a wait for Enter (`BrowserLink`: showing the link, and taking a press of Enter as a request to open it) | Decide setup answers or depend on setup/container logic | none |
| `kkt-browser` | Opening a link in the person's browser with the system's own opener (`open` on macOS, `xdg-open` elsewhere): `SystemBrowser`, with its own methods (start opening it now; say later, once, if that failed): starting the opener, reaping it on a thread, and keeping a failure to report once. It declares no contract; the program adapts it to kkt-containerized-service's `Browser` | Wait for the browser; repeat the link in a message | none |
| `kkt-rolling-file` | A file on disk that rolls over at a size and prunes old copies by age (`RotatingFile`, opened by `open_log_file` after checking its `LogPolicy`), with the clock the program gives; the rules that keep it bounded, as pure plans it carries out itself; its refusals as values (`OpenError`, `PolicyProblem`, `PolicyValue`), which the caller words. The server writes its log to one; it does not implement kkt-log | Read a real clock itself; start a logging library; know log levels; name the caller's settings | none |
| `kkt-log-tracing` | kkt-log's contract over `tracing` (`TracingLog`), and starting the tracing subscriber and telling whether it started (`logging_started`), to stdout, stderr or a writer it is given, with the program's own lines picked by the targets it names (`init_logging`, `init_stderr_logging`); the log level setting it starts with, read into kkt-log's `Level` (`parse_level`), and its defaults (`DEFAULT_LOG_LEVEL`, `DEFAULT_DEPENDENCY_LOG_LEVEL`) | Open, roll over or prune log files; name a program's settings or targets | none |
| `kkt-deployment` | What follows from how kkt is deployed on this machine, as a service, in Docker or as a client: the facts it declares (`Deployment`, `RecordedDeployment`, `ServerVersion`, `UnrecordedDeployment`, `DeploymentEndpoint`); where kkt keeps its data and its Docker files (`data_dir`, `default_db`, `docker_folder`) and a Docker install's address (`docker_url`), from kkt-machine's `Env` and `Platform`; whether `kkt serve` may run here (`guard`, refusing with a `GuardError`); what `kkt status` says and does (`status`, `status_looks_for_unrecorded`, `Status`, `StatusThen`); what `kkt upgrade` does (`running_version`, `upgrade_plan`, `upgrade`, with `RunningVersion`, `UpgradePlan`, `Replace`, `Upgrade`) and why it refuses (`UpgradeError`); this machine's key files (`keys`): where each is for each deployment, from the facts it declares (`ServerKeys`, a server's deployment and its access key file; `UnfinishedSetup`; `KeyKind`, `KeyPlace`, `KeysOff`, `Here`, `recorded_key`, `operator_key`, `configured_key`, `docker_key_copies`), the order of the reads that finding one makes, over the three kinds of read it declares, each finder taking only those it makes (`find_key`, `find_operator_key`, `find_here`, over `RecordedServerRead`, `ServeConfigReads` and `UnrecordedReads`), what `kkt key create` refuses of a key before it writes (`create_checks`, `create_destination_check`, `create_asks_for_docker`, `admin_key_collision`), writing a new key over kkt-keys' `KeyFiles` and kkt-entropy's `Entropy` (`new_key_file`), and what a person is told of an admin key's name (`admin_name_line`, `admin_name_written`), from what kkt-keys reads; why a key could not be found, placed, made or named (`FindKeyError` over its reads' `KeyReadError`, `KeyPlaceError`, `CreateRefusal`, `AdminKeyCollision`, `AdminNameLineError`, `NameNotWritten`); how a client reaches the server or the local admin endpoint (`connection`: `ConnectionSource`, which its caller implements, its answers' errors as `SourceError`, and which step failed as `ConnectionError`, with `RecordedServer` and `ServerAddress`, the servers the record names, `ConnectionOptions`, `ServeConnection`, `Connection`, `client_connection`, `admin_connection`, `admin_endpoint`, `recorded_connection`, `serve_connection`), and the one rule for a server address a client may be given (`server_address`, with `AddressError`, a kind for each refusal), which `kkt setup`, `kkt switch`, `kkt doctor` and `kkt connect` apply; what a service installed on this machine runs, from the settings it needs of the flags and the config file (`plan`, with `Request`, `ServiceSettings`, `Plan`, `PlanError`), over kkt-bare-service's `Spec`, and whether its server could listen on its address (`check_bind`, from the program's try, refusing with a `BindError`), both on plain values | Know the setup record: its caller hands it the facts | none |
| `kkt-setup` | The record of how this machine is set up (its fields, text form and place: `Record`, `record_path`, `unfinished_path`), what is wrong with a record or note that cannot be read or written (`RecordError`, with `SetupFile` and `RecordProblem`, and `RenderError`), and what to do with none (`NoRecord`) and the conversation that produces it: `kkt setup` (`run`, over the `Steps` it declares), `kkt switch` (`switch`, over `SwitchSteps`) and adopting a server installed with no record (`adopt`); whether a record's version is the one that says it is not known (`version_not_known`); `ask::Prompter` applies the question rules over caller-supplied `Lines` | Read files or run anything directly: the program carries out, or passes in, what is decided here | `testing`: `testing::DEFAULT_SETUP_URL` and `DEFAULT_SETUP_KEY`, the defaults setup's client questions accept |
| `kkt-container` | A Compose project, as a contract for any program, which knows nothing of what runs in it: `Projects` (whether the tool is there, asked without the project's variables, its name and how a person types its commands, the project's files and folders, starting, stopping, pulling and restarting its services, which are running, a command in a running service or in a new container of one, copies in and out, and how a running service was started), with `Folder` (the project's folder), `Project` (the folder and the variables every command runs without), `ProjectFile`, `Output`, `User`, `FileState`, `Ran`, `ServiceConfig` and `ProjectError` (among its kinds `NoContainer`, a service the tool names no container for) with `FileAction`. A platform crate: it depends on no other crate of the workspace | Let a tool's details into the contract; know what runs in a project | `testing`: `MemoryProjects`, Compose projects in memory with commands in a service answered by the test (`Request`) |
| `kkt-container-docker` | Implements kkt-container's `Projects` directly with Docker Compose (`Docker`), depending only on platform crates: each operation one `docker compose` command in the project's folder, without Compose's own variables or the project's shielded ones, and the project's files, tested over a private runner and a fake host | Know what runs in a project | `testing`: `testing::Scripted`, a host that answers commands from a script and keeps a transcript of what was run, made, written and said, on which a test's other parts make folders and write files with the same lines (`make_folder`, `write`, `has`), and `Docker::scripted`, a `Docker` over one, for the program's pins of the container install (`tests/container_install/`) |
| `kkt-containerized-service` | kkt's server installed as a containerized service: `ServiceContainer`, the contract its install commands use, with its records (`Inputs`, `Sharing`, `KeyCopies`, `Provisioned`, `State`, `Login`, `Existing`, `KeyCopy`, `InServer`, `Arg`, `Carry`), its error (`ContainerError`, the kinds of failure, each worded for the person: the tool not available or missing, a command that could not run or failed, a file, an unexpected answer, no container, a key copy, the start, the login or HTTPS, a path that is not plain, an incomplete install, and a failed upgrade not put back) and what it is given (`ConfigReader` with `ContainerConfig`, `LoginLink`); and `Containerized`, which implements it over kkt-container's general `Projects`, with what is kkt's in such an install: the Compose files that come with this program (the repository's own), the settings file's name, the variables it sets, which every command runs without, and what the managed folder's files say (whether the settings file is there, the version it selects, the install it describes); starting and stopping the service; the Tailscale auth key a person names; the service's status, with whether the Tailscale container is logged in and the name pack the running server uses (from serve's flags, its environment and its config file); running the server's own program in its container with a private work folder carried in or out, and reading a file under its config home in a new container; backups, in the managed backups folder and copied where the person asks; comparing the web key's copy on this machine with the server's; and setting the service up: its files and settings file, the backups folder's owner, starting it, the Tailscale container's login when it is shared (the waits and polls, the link shown once and opened on request, a restart for a node with no link, and why a login cannot finish), and the server's keys' copies on this machine, each replaced, written through kkt-keys' `KeyFiles`, with no key in a command, a line said or an error; giving the server an admin key for `/mcp-admin` and restarting it; and upgrading it (a backup first, this program's Compose files, the new version in the settings file with its other lines kept, the new image and a start, the settings put back when the pull or the start fails, then the key copies saved again, or the person told how to). It says what the person must know through the `Tell` it declares, asks and reads what a shared install's sharing node says of itself through the `SharingNode` it declares, which gives the status command and reads its answer (with `NodeLogin` and `NodeStatus`; the program adapts kkt-remote-access-tailscale's command and reader to it), opens a login link in the `Browser` it declares (the program adapts kkt-browser's `SystemBrowser` to it), and takes the folder the person ran kkt in from its caller, read once | Name a container tool: it takes the tool's name and how its commands are typed from `Projects`; touch the machine except through `Projects` and `KeyFiles`; depend on an effect crate | `testing`: `Memory`, installations in memory, with `Run`, a run in the server as it was asked |
| `kkt-bare-service` | The project's `Service` contract for running kkt's server directly on the machine as a service of the user's login (install, check it started, start, stop, status, logs, add a key, uninstall), with its values (`Spec`, `ServiceState`, `Settings`) and errors (`ServiceError`); `Unsupported`, for a system with no service manager kkt works with; the default settings a service runs with (`Settings::DEFAULT`) | Know a service manager, its commands or file formats; touch the machine; work out what the service runs: kkt-deployment does | `testing`: `Memory`, a service in memory |
| `kkt-bare-service-launchd` | Implements `Service` directly as a launchd user agent on macOS, running the server directly on the machine: the plist, `launchctl`'s commands and answers, the log file and the retries while launchd removes an old service, with a private system for its own tests | Decide what the service runs | none |
| `kkt-bare-service-systemd` | Implements `Service` directly as a systemd user unit on Linux, running the server directly on the machine: the unit file, `systemctl`'s and `journalctl`'s commands and answers, with a private system for its own tests | Decide what the service runs | none |
| `kkt-remote-access` | The `RemoteAccess` contract and `Node`, and its errors: `SharingError`, the kind a caller can act on (`Sharing`) with the implementation's words; with its testing stand-in, which gives the same kinds | Let tool details into the contract | `testing`: `Memory` |
| `kkt-remote-access-tailscale` | Implements kkt-remote-access's `RemoteAccess` directly over the host's Tailscale login (`Tailscale`, made with the command a person should rerun once they have fixed what a refusal names, `Tailscale::new`): `tailscale`'s commands and what their answers mean, tested over a private runner; and the status command (`Tailscale::STATUS_COMMAND`) and reading its answer, which a caller got wherever Tailscale runs (`Tailscale::read_status`, into its own `Status`, which says whether the node runs or waits for a login, or a `StatusError`); each refusal is a `SharingError` of its kind, worded with Tailscale's advice | Change login, DNS preferences, unrelated sharing, or anything of the caller's | none |
| `kkt-llm-agent-config` | Client MCP configs and their file formats, with the environment variables naming the clients' own folders, which the program asks the environment for (`ENV_FOLDERS`); when a client's settings are changed through its own command (Claude Code), asking the `ClientCommand` it declares, whose failures are a `CommandError` (a `CommandFailure` kind and the implementation's own error) (the program adapts kkt-llm-agent-command's `Claude` to it), and restoring the file if it fails; its errors (`ClientConfigError`, with `FileAction`, `Folder`, `Shape`, `TomlProblem`, `Text`, `Encoded` and `Unresolved`), each kind worded for the person | Touch the machine except through `Host` and `ClientCommand` | `testing`: `Recorder`, a stand-in `ClientCommand` that fails with the `CommandFailure` it is given |
| `kkt-llm-agent-command` | Claude Code's own command line: `Claude`, made with the server's name and what to retry with (`Claude::new`), with its own methods (remove the server, add it as Claude Code's own entry): `claude mcp remove` and `add-json` at user scope, `CLAUDE_CONFIG_DIR`, and why they failed (`ClaudeError`), worded for the person, tested over a private runner. It declares no contract; the program adapts it to kkt-llm-agent-config's `ClientCommand` | Echo the command's output, which may hold credentials | none |
| `kkt-diagnostics` | `kkt doctor`'s checks and their plan; its lines about how this machine runs kkt and the fixes that follow, from the facts it declares (`this_machine`: `Setup`, `ServiceSeen`, `DockerKkt`, `TailscaleLogin`, `WebKeyCopy`, `UnrecordedServer`; `server_fix`, `web_key_fix`, `admin_key_fix`, `service_check`, `docker_check`, `tailscale_login_check`, `docker_web_key_check`, `unrecorded_check`), quoting the paths in a fix with its one rule (`shell_word`); setup's wait for a new server; the `Prober` it declares (`Probing`), with the facts it reports (`KeyFile`, `Answered`, `Initialized`, `Status`, `Reply`, `ReplyError`, and `Unprobed`, why no probe could be made, worded for the person, which the checks return as their error), from which it makes the checks, their names and fixes (`Check`); the program adapts kkt-probe's `HttpProber` and kkt-keys' `KeyFiles` to it | Change anything; touch the machine except through its `Prober` | `testing`: `Memory`, key files and answers in memory |
| `kkt-probe` | Probing an MCP server over HTTP with reqwest: `HttpProber`, with its own methods (`answer`, a plain GET; `initialize`, an MCP `initialize` that closes the session it opens) and the caller's `Settings` (timeout, size limit, protocol version, client name and version); what it finds, as plain facts (`Answered`, `Initialized`, `Status`, `Reply`, `ReplyError`, `Unprobed`). It declares no contract; the program adapts it to kkt-diagnostics' `Prober` | Let reqwest types out of the crate; judge or word what it finds | none |
| `kkt-backup` | Backups, as a contract: backup names, contents, modes and pruning choices, as immutable layouts; which server `kkt backup` acts on (`backup_of`, from where the caller says this machine's server is, `ServerHere`), which key files a local backup holds (`backup_keys`, with `AccessKeyFrom`, `BackupKeys` and `FileThere`), and where backups go (`upgrade_backup_folder`, `service_backup_folder`); retention `Settings`, its default and bounds (`Problem`, `Settings::problems`), and its run-record line (`Settings::record_lines`); and `Backups`, writing a layout into a folder and pruning old backups there; its errors, each worded for the person (`BackupOfError`, `FolderError`, `InspectError`, and `BackupError` with `FileAction`). kkt-backup-os implements `Backups` | Read the clock or copy the database itself: the caller passes both | `testing`: `Memory`, backups in memory |
| `kkt-backup-os` | Backups on disk: `OsBackups` implements kkt-backup's `Backups`, writing its layouts and removing the old folders it picks, with the database copy the program gives it, which gives its own error as the cause (`CopyError`) | Choose backup names, contents, modes or retention | none |
| `kkt-lane-files` | Lanes as files: the `<lane>.md` format, and reading and writing a folder of them, with the commit it is at from the `LaneCommit` it declares (the program adapts kkt-revision's `Git` to it); its errors, each worded for the person: what is wrong with a lane file (`ParseError`, with `HeaderKey`), and which step of reading or writing a folder failed (`ReadError`, `WriteError`) | Touch the machine except through `Disk` and `LaneCommit`; know a version-control tool | `testing`: `Memory`, commits in memory |
| `kkt-revision` | The commit a folder is at, as the machine's git says: `Git`, with its own `commit` method: git's commands, what their answers mean (expected absences, the dirty suffix) and its failures' wording, tested over a private runner. It declares no contract; the program adapts it to kkt-lane-files' `LaneCommit` | Read or write the folder's files | none |
| `kkt-watch` | `kkt watch`, and why it stopped (`WatchError`), worded for the person, with the errors of the place and the report it is given (`PlaceError`, `ReportError`) passed on as their implementations made them | Reach the server, the time, its place or its output except through what is injected (the server as kkt-agent-client's `Watched`) | none |
| `kkt-agent-client` | A client of kkt's server, as a contract in kkt's terms: `Servers`, which makes `Avatars` (register, reclaim, describe) and `Watched` (an agent's or the admin's view: handle, channels, latest ID, a message, a wait) for an `Address`; their records and `Failure` | Name a transport, a protocol library or its errors | `testing`: `Memory`, a server in memory |
| `kkt-agent-client-mcp` | Implements kkt-agent-client directly over MCP through rmcp and reqwest (`Mcp`, `Client`, `Remote`, and an admin's view of its own), and plain HTTP requests for a program's test client (`Http`, `HttpReply`); what the watch asks for at a time (`Settings`), as plain values | Let an rmcp or reqwest type out of the crate | none |
| `kkt-agent-files` | Agent credential files and `kkt watch`'s state files on this machine: reserving and saving a token or recovery code, reading a token file under the caller's credential-file rule, file locks, claiming a state file; why a credential or state file could not be used (`AgentFileError`, with `CredentialAction`, `StateAction` and `LockError`), worded for the person | Choose the credential-file rule or where its warnings go | none |
| `kkt-agent-commands` | The agents' commands: `kkt register` and `kkt reclaim` (with `NewAvatar`, and the wording of what the server refuses), `kkt watch` (`watch::run`, `WatchTask` and its timed-out exit code) and `kkt connect`; how a client reaches the server or the local admin endpoint, through kkt-deployment's `ConnectionSource`; `credential_file_check`, which decides an agent credential file's ownership and permission tightening (`ForeignCredentialFile`); why a command failed (`AgentCommandError`), worded for the person, with the errors of its own contracts (`CredentialsError`, `AdminKeyError`, `ProgramError`) passed on as their implementations made them; `AgentParts`, the effects and facts the program passes in, as its own contracts (`AdminKey`, `Credentials`, `KeyText`, `Program`, `Output`) and kkt-agent-client's (`Servers`), kkt-watch's and kkt-llm-agent-config's | Depend on an effect crate (an `-os` crate or a contract's implementation) or on another assembly; touch the machine except through `AgentParts` | none |
| `kkt-hash` | Hashing: `sha256`, the SHA-256 digest of some bytes; the one crate that uses sha2 | | none |
| `kkt-scratch` | Tests' scratch folders under `target/tmp` | Be anything but a dev-dependency | none |
| `kkt-api-sim` | A simulated application, for tests and the web preview: `Preview`, the kkt-api crates' operations the web page uses over a world made up from a seed, answering `limits` with its own values, and `PreviewGate`, a stand-in for kkt-web's `Gate`, open or with an access key checked as kkt-admission checks one | Be in a release | `testing`: all of it |

**Known exception: the setup record holds facts about many subjects.** kkt-setup's `Record` is the one file that says how this machine is set up: the kind of deployment, its server's address and access key, and which server its clients use. These facts concern several subjects, which departs from a module's unitary concern (ARCHITECTURE 1.1). They stay in one record because a person reads it and `kkt setup` writes it as one file, and existing installs depend on its form. Only the program and kkt-install-commands read it, and they hand each crate only the facts it declares in its own terms (ARCHITECTURE 3.5.8.2). The exception would end if each crate described the part of the record it owns, and kkt-setup composed those parts into the one file without knowing what they hold.

**Known exception: the config file holds settings for many crates.** kkt-config-file's `Config` is the config file as a person writes it, with settings that belong to many crates, which departs from a module's unitary concern (ARCHITECTURE 1.1). It stays one file because a person edits one file, and existing installs depend on its form. Only programs depend on kkt-config-file. Each crate declares the settings it takes, with their defaults and checks, and the program hands each crate its own (ARCHITECTURE 3.5.8.2). The exception would end if each crate described its own settings, their names, defaults and checks, and kkt-config-file read the file into them without knowing what they hold.

The program, in the root package:

| Part | Owns | Must not | Features |
|---|---|---|---|
| `src/bin/kkt/` (`main.rs`, `cli.rs`) | The `kkt` command: subcommands and the files and environment they read; `cli.rs` wraps clap and holds every clap type; `main.rs` builds kkt-install-commands', kkt-operator-commands' and kkt-agent-commands' parts in one place (`NativeParts`, `agent_parts`) and runs each command through them | Pass a clap type out of `cli.rs` |  |
| `src/machine/` (in the library `src/lib.rs`) | What kkt keeps on this machine: serve settings (`settings`: `ServeOptions`, `ServeSettings` and `server_settings`, from the flags, the config file and each owner's defaults) and the config file, the setup record, key files, how a client reaches the server (`connection::ConnectionReads`, kkt-deployment's connection question answered here, with the record's servers read through kkt-install-commands' `deployment::recorded_server`), the server's key reads (`keys`), this platform's service manager (`service::for_this_os`, picking kkt-bare-service-launchd, kkt-bare-service-systemd or kkt-bare-service's `Unsupported`), a shared install's Tailscale login read through kkt-remote-access-tailscale's reader (`sharing_node::TailscaleNode`, kkt-containerized-service's `SharingNode`), and the adapters the three assemblies take: kkt-install-commands' `InstallParts` and kkt-operator-commands' `OperatorParts` (`native::NativeParts`, with the serving facts their questions ask, `native::HereLanes`, the server `kkt lanes` acts on, picked from what is installed here; `native::InstalledOnThisMachine`, kkt-install-commands' reads of what is installed here as kkt-operator-commands' `InstalledHere`; and `native::SetupLanes`, the operator commands' lane check and import as the install commands' `ImportLanes`, on a server picked the same way) and kkt-agent-commands' `AgentParts` (`agent_parts`, which injects kkt-agent-client-mcp's `Mcp` as it is), among them kkt-agent-commands' credential-file rule over kkt-agent-files' credential and watch-state files. The setup record's readers are kkt-install-commands', and the operator's key finders kkt-deployment's | | |
| `src/commands/` (in the library) | `serve`, which composes the server over kkt-server (with the backup before an upgrade), and the adapters two assemblies' commands take: `lanes`' disk and git (`LaneDisk`) and kkt installed in a container as the lane commands take it (`LanesInContainer`, over kkt-containerized-service's `ServiceContainer`, the install's folder and the container's config reader), and `watch`'s output (`WatchOutput`). The install commands are kkt-install-commands', the operator's commands kkt-operator-commands', and `kkt register`, `reclaim`, `watch` and `connect` are kkt-agent-commands'; the program's tests of them over its real parts sit here and in `src/machine/` | Take a clap type | |
| `src/terminal/`, `src/process.rs` (in the library) | Adapting Terminal to setup's Lines and the terminal's BrowserLink to the container's LoginLink; this program's path over kkt-machine's `Machine`, and how kkt words a failure to find it (`kkt_path`) | | |

A second program, `programs/kkt-keymode` (package `kkt-keymode`), assembles key mode again from the shared crates with none of the `kkt` program's code: the same config file through kkt-config-file over kkt-files, into settings of its own, the same key files through kkt-keys and kkt-admission's checks, and kkt-server with kkt-identity-keymode's `KeyMode`. It is for development only and never released (`publish = false`, `dist = false`). It makes no key, backup or log file and adopts no old database. The HTTP-only end-to-end tests (e2e, admission, web_login, admin, watch, latency, contract) run against it and against `kkt serve` in CI: `KKT_TEST_PROGRAM=kkt` or `kkt-keymode` makes `tests/common`'s `start` and `start_with` run that program in a process of its own (build kkt-keymode first). Unset, they run the server in the test's process. A change to `kkt serve`'s settings or startup that the tests see belongs in both programs, the order its settings are checked and recorded in (`CHECK_ORDER`, `RECORD_ORDER`) among them.

The fake product, `programs/kkt-fake-product` (package `kkt-fake-product`), is the shared crates assembled as a product other than kkt would assemble them, with two swarms, `sw_main` and `sw_other`. Callers are let in by any bearer token (`Admission::Bearer`), and its `FakeIdentify` decides who each made-up bearer is: one swarm's agents, its operator on the web page, or its admin at `/mcp-admin` (`AdminAdmission::Bearer`). Its tests (`tests/inventory.rs`, `tests/slow_subscriber.rs`, `tests/refusals.rs`) start it in the test's process with `kkt_listen::listen` and use it over HTTP. They seed both swarms, read everything each swarm's agents, operator and admin can read, and check that nothing of the other swarm appears, and that each swarm's reads are the same byte for byte whether or not the other swarm was seeded. They also check that a slow subscriber in one swarm does not lag while the other floods. An unknown bearer, or one used for the wrong role, gets nothing on `/mcp` or the web page. Its HTTP/SSE caller lives only in `tests/client/`, through kkt-agent-client-mcp's `Http`; the library has no HTTP client. Its program serves it over a new database file through `SqlitePlace::create` and writes through `Terminal`, to look at by hand. It is dev-only and never released, like kkt-keymode.

The web preview, `programs/kkt-webapp-preview` (package `kkt-webapp-preview`), serves kkt-web's page alone (`kkt_server::page`) over kkt-api-sim's simulated application and stand-in gate, with the real clock, randomness and stop signals, so the page can be worked on with no database, MCP or agents (`docs/development.md`). It turns on kkt-api-sim's `testing`, as a dev-only program may. It is dev-only and never released.

Other files: `docs/architecture/` (the interactive crate map; run its `generate.py` after a change to crates, dependencies or public items. CI checks it is current only on pull requests into `main`, so before a release, regenerate it on `next`. CI runs its page test, `docs/architecture/run-page-test.sh`, on every pull request; run it after a change to the page or the map's data); `tests/migrations/sql/` (the SQL migrations of releases up to 0.10.0 and of `next`'s swarms, from which the tests build older databases);
`tests/container_install/` (the container install's pins: the scenarios, what the scripted host answers in each, and what the install does, a transcript a file, which `main.rs` checks; `UPDATE_GOLDEN=1` writes the transcripts again, and a change to any file there is a change to existing installs); `tests/container_install_subjects/` (the `ServiceContainer` implementations those pins run, each against the same transcripts);
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
true` (`kkt-scratch`, `kkt-api-sim`, `kkt-keymode`, `kkt-fake-product`, `kkt-webapp-preview`) is never part of a release:
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
- Dependencies are injected through `kkt_rules::Deps` (`kkt_rules::Startup` at startup, where `start` records the run and builds the `Rules`), `kkt_server::Parts`, `kkt_install_commands::InstallParts`, `kkt_operator_commands::OperatorParts` or `kkt_agent_commands::AgentParts`.
- Settings and defaults live with the crate that owns them: each crate
  declares the settings it takes, with their defaults and checks, and there
  is no shared configuration crate. Each program gathers a server's
  settings from flags, the config file and those defaults (kkt's in
  `src/machine/settings.rs`), and hands each crate its own. Every setting is checked at startup. Settings,
  which can differ from one server run to the next, appear in the run
  record, with each owner wording its lines and each program putting them
  in its own order, as it orders the startup checks (kkt's in
  `src/machine/server_run.rs` and `src/machine/settings.rs`). Fixed constants do not: the run's recorded
  commit fixes them. Bad settings stop the server with an error naming the
  setting.
- Rules return `RuleError` with a fixed code from `crates/kkt-api-agent/src/error.rs`.
  `unwrap` and `expect` are for tests only.
- Logging happens only in side-effect and composition code (ARCHITECTURE.md 4.2.1). A
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
- axum is the HTTP framework of `kkt-web`, `kkt-mcp`, `kkt-server` and `kkt-listen` only; its types do not leave those four, but for the router a program takes from kkt-server's `App` (`into_router`) and hands to kkt-listen's `listen` without naming it.

**Public surface.** Make an item public only when another crate or the program needs it.

**Third-party tools and libraries.** A third-party tool or library is wrapped by a crate named for what it
does for kkt, with a contract in kkt's terms. With one implementation, the
contract and the implementation are two crates, or the consumer declares
the contract (the adapter model), as "Kinds and features" says. The
implementation owns its tool's logic, tested against a private runner.
Either way the program injects it. Language-shaping libraries retain
RUST.md rule 7's exemption.

**Dependencies between crates.** Add one only for a real reason. A crate
that depends on another for something minor, such as one small helper, is
a sign the crate lines are wrong.

## Tests

- Run `cargo test`, `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt --check` before every commit. CI runs the same on Ubuntu and
  macOS, and a pull request merges only when they pass.
- CI also builds a crate outside the workspace that depends on
  kkt-api-agent, kkt-rules, kkt-server and kkt-listen, whose dependencies have
  build scripts, by git at the commit under test, as kkt-pro will
  (`.github/scripts/build-outside-crate.sh`): a shared crate must build
  when used that way.
- CI also checks the crate rules. check-crates.sh fails on a crate
  other than a program that depends on an effect crate (unless listed in
  effect-exceptions.json), on a crate other than a program that depends on
  a composition crate in [dependencies], on a platform crate that depends
  in [dependencies] on a crate outside the platform layer, and on any
  `real` feature. `.github/scripts/check-platform-words.py` fails when a
  platform crate's code outside its tests says `kkt` or `kashkeshet` in a
  string or a public item's name: the caller passes kkt's words in. It
  also fails on a map description in concerns.json that names kkt in the
  platform layer, gives history, says what a crate is not, or runs past
  one to three short sentences. It holds each crate's header (the `//!` lines
  opening its lib.rs or main.rs) to the same rules: no history in it, and
  no negative in its first sentence, which the map shows.
  `.github/scripts/check-lint-exceptions.py` fails on an `allow` attribute
  and on an `expect` with no reason: a lint exception is
  `#[expect(.., reason = "..")]` (RUST.md 9).
  `docs/architecture/check-programs.py` builds the map from the code and
  fails on a kind that does not fit, a broken rule, and machine work in a
  program's own code that the temporary-breaks table in
  `docs/architecture/refactor-plan.md` does not list.
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
