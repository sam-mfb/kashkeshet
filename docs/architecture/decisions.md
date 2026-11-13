# Decisions behind the proposed map

The proposed map (the "Proposed" mode of `index.html`) states what the
workspace is to become. This file records how it got there: the decisions,
with their reasons, the assumptions that were accepted, and the options that
were rejected. The page does not link here; `proposed.json` keeps the ids its
checks need.

Message numbers are given only where they help to find a discussion in
#kkt-dev.

## The rule

- **One dependency-injection rule.** Every trait a crate depends on is a
  contract, declared by the crates that use it. A contract with more than two
  users lives in a contract-only crate of its own; one with two or fewer is
  declared by each user (msg 1938).
- **The strict reading.** No implementation implements a contract another
  crate declares for itself; the program adapts the implementation to it.
  Letting an implementation meet its one consumer's contract directly was
  rejected, because it couples the two (msg 1978).
- **A crate may implement its own traits.** FixedClock implements Clock,
  DryRun implements System, and kkt-db's own backends implement kkt-db's
  traits. Without this, every test double would need an adapter.
- **Shared contracts are implemented directly.** Implementing a shared
  contract crate's contract is like consuming it, so kkt-db-sqlite implements
  kkt-db's traits, SystemClock implements Clock and OsEntropy implements
  Entropy, with no adapters. The strict reading covers only contracts a
  consumer declares for itself (msg 2020).
- **What counts as use.** A crate uses a contract only when its code calls the
  contract's methods. Naming a trait to wire an implementation in, or to
  implement it in an adapter, is not use. On this count no contract crossed
  the line of more than two users in either direction (msg 2027).

## Kinds and layers

- **Every crate has a kind, by what it does**: contract, logic, effect,
  composition or test support, plus mixed in the current map only. There is
  no configuration kind: it named a domain, not what a crate does (msg 2111).
  A contract may hold pure behaviour on its own value types (Time arithmetic,
  query checks), but no effect and no wiring. Only programs and compositions
  depend on effect crates; no effect crate depends on another (msg 2170).
  Other crates' tests use stand-ins instead (msg 2150, the audit's F16): kkt-web's and
  kkt-web-preview's tests take the fixed clock, seeded entropy and a manual
  stop, while the preview program keeps the real ones. propose.py checks dev
  arrows too.
- **No crate mixes logic or a contract with an effect.** So the effects split
  out: listening moved from kkt-server to kkt-server-listen, writing and
  pruning backups to kkt-backup-os, and the stop signals to kkt-shutdown-os.
  Each OS implementation moved to its own `-os` crate.
- **kkt-server-listen may use axum.** It serves the router kkt-server builds.
  CLAUDE.md will name it with the code refactor.
- **Layers say where a crate sits**: programs, assembly, surfaces, access,
  core, data, operations, clients, foundations and dev-only. kkt-search sits
  in foundations, as pure text the data crates build on.

## Adapters

- **Adapters were measured, then the contracts reshaped** (msg 2009). Every
  adapter method was classed as forwarding (the provider's method of the same
  name, at most renamed) or translating. As first charted, all 198 adapter
  methods only forwarded: each provider had copied its consumer's interface.
  The reshapes below brought them to 53 methods, nearly all of them adapters
  whose provider exists only to meet one contract.
- **kkt-server writes the adapters the three server programs share**, once,
  as each program would, provided it decides nothing (msg 2019). The web gate
  and the notices' wording are its adapters.
- **No shared adapter crates** (msg 2045). kkt-store-rules and
  kkt-db-sqlite-db went once the reshapes made them unnecessary.

- **Three consumer-declared contracts stay as they are** (msg 2139, the
  audit's F13). kkt-watch's Server, kkt-web's Gate and kkt-service's System
  each have one consumer and two implementations, and stay declared by their
  consumers, with the program's adapters as today. A judgment call under
  template 3.3: for Server, two implementations in one crate do not justify a
  contract crate, and the adapters are fine for now; for Gate, the second
  implementation is a stand-in (kkt-api-sim's PreviewGate); for System,
  DryRun is kkt-service implementing its own trait.

## Storage

- **kkt-store merges into kkt-rules** (msg 2017). kkt-store made decisions of
  the rules' own (the swarm checks, lane version numbering, the channel scope),
  so it was not just an adapter. kkt-rules uses kkt-db directly, and the
  separate Reads and Writes contract, with its copied record types, goes.
- **The database stays private to kkt-rules** (msg 2032). kkt-rules' `start`
  records the server run and then builds the Rules, because the run's id goes
  into the Deps that build them; it could not be an operation of the Rules
  themselves (msg 2042). The 0.10.0 migration test moves into kkt-rules' tests.
- **kkt-db is a contract only** (msgs 2012 and 2014). Database goes: its typed
  read and write are provided methods of BackendExt, over Backend's async read
  and write, and the thread pool is kkt-db-sqlite's. migrate and pending are
  Evolve's provided methods, and the query checks are Schema's methods,
  because which queries are valid is part of the contract every backend
  honours.
- **Search's text functions move to kkt-search.** Search and Hit stay in
  kkt-db, and so does Excerpt, as part of what a caller asks for; kkt-search's
  excerpt takes its marks and window as arguments.
- **kkt-db's Legacy has no backend's terms** (msg 2143, the audit's F15). It
  describes a database from before migrations were tracked; reading SQLite's
  user_version stays in kkt-db-sqlite. Legacy is not behind real, and
  propose.py refuses an item behind real in a contract crate.

## The application's operations

- **kkt-rules' operations are shared contracts** (msg 2031): they are the
  heart of the application, and the web page and the MCP endpoints are today's
  protocols over them. kkt-mcp and kkt-web call them directly and depend on no
  kkt-rules type; kkt-web's copied records, Source and Sources go.
- **Three crates, one per level of authority** (msg 2045): kkt-api-agent (what
  any registered agent may do, and the shared records), kkt-api-operator (what
  the human who runs the server may do) and kkt-api-admin (the admin reads and
  creating a swarm). Operations are sorted by who may call them, not by topic:
  one that takes an agent is the agent level's, one that takes the operator is
  the operator's. The cases that needed judgement were accepted as reasoned:
  set_purpose, register and reclaim, the current announcement, in_swarm and
  limits sit with the agent level; subscribe, the lane operations /mcp-admin
  calls and the changes both the page and /mcp-admin make sit with the
  operator level; AgentView is a shared record (msg 2052).
- **Open: who may change what is server-wide.** Lanes, the server announcement
  and server runs belong to the server, but the operator level changes them.
  Where several swarms each have an operator, changing them may belong at the
  admin level, with reading staying at the operator level (msg 2052).
- **The operator flag is part of the records.** Whether a message's author,
  or a channel's last speaker, is the operator is filled in by kkt-rules,
  which knows who the operator is, so every surface shows the same answer.
- **Lanes are saved in two ways.** The page creates and updates a lane with
  separate operations, so its source chooses nothing; agents and /mcp-admin
  keep save_lane, which creates or updates (msg 2019).
- **Limits.** The limits the rules enforce come from the agent level's
  `limits`; the page's own (messages per page, excerpt length) are kkt-web's.
  The default name pack is kkt-settings', so kkt-container needs no API crate.
- **The overview takes the activity chart's shape as arguments** (msg 2140,
  the audit's F4). kkt-api-operator's Views::overview takes the bucket size
  and the window. kkt-web passes its own WEB_ACTIVITY_BUCKET and
  WEB_ACTIVITY_WINDOW, which stay in kkt-web, and kkt-rules no longer depends
  on kkt-web.
- **A contract crate holds no configured values** (msg 2138, the audit's
  F14; template 3.2.5.1). kkt-rules' Settings, the limits the rules enforce,
  stays in kkt-rules, and the program fills it. kkt-api-agent keeps only the
  record Api::limits returns, Limits. MAX_LANE_DESCRIPTION_CHARS becomes one
  of kkt-rules' settings, with its default gathered with the others, and
  limits reports it. The simulated implementation answers limits with its
  own values.

## The web page and identity

- **Web sessions move into kkt-web** (msg 2021). kkt-admission keeps the
  access decision; the gate asks it only whether a key is good and who a
  viewer is. A session holds the gate's own value, since kkt-web cannot name
  kkt-admission's Principal.
- **Stand-ins live in a test support crate, kkt-api-sim** (msg 2137, the
  audit's F8). Preview, the simulated application, which answers limits with
  its own values, and PreviewGate, a stand-in for kkt-web's Gate, are behind
  its testing feature. kkt-web's and kkt-mcp's tests use it. kkt-web-preview
  keeps only the program, which turns testing on: dev tooling counts as test
  code. A test support crate may implement a contract a consumer declares for
  itself, since its stand-ins do. kkt-web's tests and kkt-api-sim depend on
  each other, which Cargo allows for a dev-dependency.
- **kkt-rules' testing items stay** (msg 2137). They help test the real
  implementation and stand in for nothing: memory is the real Rules over
  kkt-db's in-memory database, fail_next_write makes a write fail, and
  swarms_stay_apart is the check every store must pass.
- **An arrow turns on only features its target has** (msg 2137). Where a
  feature moved (kkt-clock's real, to kkt-clock-system), the arrow to the
  crate that took it turns it on; propose.py refuses any other.
- **Identity is a contract crate, kkt-identity** (msg 2135, the audit's F11;
  template 3.3.3). Several implementations are expected: key mode's, the fake
  product's and kkt-pro's. It holds one trait, Identify, as today: the web
  gate's question, which operator a principal is, is one of the three the
  endpoints ask, so a second trait would split one contract. A stand-in,
  testing::FixedIdentify, is behind its testing feature. KeyMode moves to
  kkt-identity-keymode, with its tests, and implements Identify directly, as
  FakeIdentify does. This supersedes msg 2025 (assumption A8), where kkt-mcp
  declared Identify, kkt-server declared WebIdentify and each program adapted
  KeyMode to both: kkt-mcp's Identify, WebIdentify and the programs' four
  adapter methods go.

## kkt-config

- **kkt-config is broken up by domain** (msg 1979). Format and fixed facts go
  to the crate that owns them; tuning values become settings the program
  fills. The page's kkt-config table lists every item.
- **kkt-config goes altogether** (msg 2111). Its last items go to their
  owners: the web session ID length to kkt-web, with the sessions; the access
  key's form and check, and the key files' mode, to kkt-admission; the error
  every settings check gives to kkt-settings. kkt-backup, kkt-client-config
  and kkt-container each state the mode of the private files they write,
  instead of borrowing the key files' mode.
- **kkt-settings is logic, and the config file is an effect** (msg 2111).
  kkt-settings combines the sources of settings it declares (the flags and
  the config file, flags over file over explicit defaults), checks them and
  gives each crate what it needs. A new effect crate, kkt-config-file, reads
  and parses the config file and edits its text; the program adapts it, and
  the flags, to kkt-settings' source. kkt-container reads the container's
  config through its Host and parses it through a parser contract of its own,
  which the program adapts kkt-config-file to. kkt-setup's own record is a
  different file and stays with kkt-setup.
- **kkt-settings only combines its sources** (msg 2134, the audit's F2, option
  a). Each crate declares the settings it takes and its check's error as its
  own types: kkt-admission, kkt-hosts, kkt-service, kkt-web-preview and
  kkt-config-file each have their own, and kkt-settings' InvalidSetting is
  only its own. Constants go to their owners: the config file's folder and
  name to kkt-config-file; the key files' names and paths, and the access key
  setting, to kkt-admission; the data folder and default database to
  kkt-setup; the log's settings and their check to kkt-logs; the bind check
  and address to kkt-hosts. kkt-settings combines the flags and the config
  file, through its own SettingsSource, over explicit defaults, into raw
  values, and depends on none of its consumers. The program turns the raw
  values into each crate's settings and calls each owner's check, which is
  simple adapter logic; so check_settings, which F1 had moved to kkt-settings,
  goes. No crate but the programs depends on kkt-settings. kkt-provenance
  records the run's settings as text the program words.
- **The machine's facts are a contract crate, kkt-machine** (msg 2134). Env
  holds the home folder and the folders the environment names, with Platform;
  Machine gives them, with a stand-in behind testing. kkt-client-config,
  kkt-container, kkt-service and kkt-setup take Env; each owner builds its own
  paths from it. kkt-machine-os reads the facts and implements Machine
  directly; the program wires it in and passes the facts on as values.
- **The config file's format is logic; its bytes are an effect** (msg 2136,
  the audit's F3). kkt-config-file is split as kkt-backup is: kkt-config-file
  (logic) parses the file's TOML into its own Config, with its own
  ConfigError, and edits its text, knowing nothing of kkt-settings;
  kkt-config-file-os (effect) reads and writes the file's bytes, and the
  instructions file's. The program adapts a parsed file into kkt-settings'
  source, in one translating method, and parse_file into kkt-container's
  ConfigReader.

## The programs

- **The programs do no machine work themselves** (msg 2133, the audit's F1,
  option a; ARCHITECTURE 4.3.3). kkt, kkt-keymode and kkt-fake-product keep
  argument parsing, wiring, simple adapters and thin command bodies. Their
  files, keys, credentials, terminal, environment and log go through effect
  crates, and their commands' decisions go to the logic crate for the area.
  kinds.py checks a program's own code for machine work, and propose.py
  checks that every program file doing it today says where it goes.
- **Grouped by owner, to keep the count down.** Three new effect crates,
  packaged like the -os crates (plain types the program calls or adapts):
  - kkt-admission-os: the key files, which kkt-admission checks but does not
    read. Both kkt and kkt-keymode adapt its KeyFiles to kkt-admission's
    KeyTexts, in place of their own readers.
  - kkt-setup-os: setup's own files, the setup record, its unfinished note
    and kkt's data folder; this program's path; PATH. kkt-setup decides;
    kkt-setup-os only touches. Other files go through the effect crate that
    owns their area (msg 2144): the Docker folder's .env and the Tailscale
    auth key file through kkt-container's Host, staged keys and whether key
    files are there through kkt-admission-os, whether a database is there
    through kkt-db-sqlite.
  - kkt-terminal-os: output, setup's questions, the login link and the
    closed-pipe default (libc). The commands write their output through it;
    the program adapts it to kkt-setup's Lines, kkt-watch's Report and
    kkt-container's LoginLink.
- **Existing effect crates take the rest**, since each already owns the area:
  kkt-agent-client takes agent credential files and watch's state file
  (it is the agent client's credentials; libc for the owner check moves with
  them), reading a file's owner and mode and tightening it, while the rule
  for them, refusing another user's file and tightening a readable one, with
  the advice, is kkt-setup's (msg 2144); kkt-logs takes starting the log (tracing-subscriber moves there);
  kkt-config-file takes the environment that places kkt's files, writing the
  config file and reading the instructions file. kkt-service-os tries the
  service's address, and kkt-db-sqlite opens a new database file refusing an
  existing one, for the fake product.
- **Decisions move to logic crates.** kkt-setup takes which server and key a
  client uses, the order keys are looked for, the record's rules, doctor's
  checks, the status lines, the per-server key file's name (sha2 moves
  there) and setup's prompts (its Prompter over its own Lines). kkt-settings
  takes the settings checks, which kkt-keymode copied and now shares.
  kkt-service takes whether a busy address stops an install.
- **The fake product's HTTP client moves into its tests.** Only its tests use
  it, so reqwest, serde_json and futures become dev-dependencies.

## Logging

- **Logging is a shared contract crate, kkt-log** (msg 2146, the audit's F5).
  Log records an event at a Level; a stand-in behind testing captures events
  for tests. kkt-mcp and kkt-web, the logic crates that log today, take it as
  an injected dependency and stop using tracing; kkt-server hands the
  program's Log on to them. kkt-logs-os's TracingLog implements it directly,
  and the programs wire it in. Starting tracing stays in kkt-logs-os, where
  F1 put it. This replaces CLAUDE.md's exception that logic crates log at the
  edges; CLAUDE.md changes with the refactor.
- **kkt-logs is split as kkt-backup is** (msg 2147, the audit's F7). It holds
  real decisions, not just file work: when a write rolls the file over (past
  the size, never while the file is empty), which rolled file goes, how rolled
  files are named, when to look for old ones and which are past their age.
  kkt-logs (logic) holds those rules and the log's settings; kkt-logs-os
  (effect) writes, renames and deletes the files as the rules say, and starts
  tracing.

## Third-party libraries

- **Hide them, consolidating where reasonable** (msg 2141, the audit's F6;
  RUST 7 as Sam is rewording it). The aim is that few crates import a
  third-party library, not one importing crate per library.
  - sha2 goes into kkt-hash, a small pure crate whose sha256 the six users
    call: kkt-admission, kkt-names, kkt-provenance, kkt-rules, kkt-setup and
    kkt-web.
  - reqwest is in two effect crates: kkt-agent-client, whose Http the fake
    product's tests use, and kkt-diagnostics-os, which keeps its own HTTP
    for its probes (msg 2170). An effect crate does not depend on another,
    and the revised RUST 7 leaves consolidating to judgment.
  - toml stays in kkt-config-file, for kkt's config file, and in kkt-setup,
    for its own setup record and unfinished note, which are different files
    (msg 2111). toml_edit stays in kkt-client-config, for other tools' files,
    and in kkt-config-file.
  - These stay as they are: tokio and serde_json, which shape the language;
    axum, the web framework, in the crates CLAUDE.md names; rmcp and
    schemars, MCP on both sides; and tokio-util.

## During the refactor

- **The proposed map is updated as we go** (Sam, 2026-11-11, msg 2357). A
  step that differs from the proposal on purpose adds a row to the
  differences table in `refactor-plan.md`; once it merges, the difference is
  charted into the proposal, so the page shows it. The table says which
  differences Sam has accepted. The first to be charted are 1.9's single
  watch adapter, 2.2's own service output, 2.6's git contract and 3.1d's
  program arrows to the formatter's and the page's settings.

## After the refactor

After step 8.1, Sam decided three more phases (2026-11-12): kkt-settings
goes (phase 9), the program's tools become two assemblies, kkt-operate and
kkt-agent-commands (phase 10), and kkt-db gets a fast test stand-in, so
kkt-rules' tests use no effect crate (phase 11). All three are done, and
charted into the proposal.

The two differences left at step 8.1 are gone:

- **The arrow kkt-server → kkt-api-admin is there** (step 9.2, msg 2837).
  `Parts` takes the contract, an `Arc<dyn AdminApi>`, and kkt-server no
  longer depends on kkt-rules in its code.
- **kkt-rules' tests no longer depend on kkt-db-sqlite** (steps 11.1 and
  11.2, msg 2805). They run on kkt-db's `testing::FastPlace`, and the
  program's tests run the store and a chosen part of the rules' scenarios on
  kkt-db-sqlite.

Then Sam ruled that every third-party dependency is split: a contract crate
named for the scope declares what kkt needs in its own terms, and a wrapper
crate named for the tool implements it directly, holding the tool's
knowledge and the code that runs it; the program only injects the wrapper
(msgs 3041, 3047, 3050). Phase 12 did that for Tailscale, Docker, launchd
and systemd, the MCP client, git, the doctor's HTTP probe, Claude Code's
command and the system's browser opener. The adapters the program still
holds reach only the machine's own files, terminal and paths, or its own
serving code; the plan lists each one and why.

The final scorecard, on the code at the end of phase 12 (1007e1a):

- crates: 72 of 72 proposed crates exist; none missing; none to remove;
- items: 794 of 794 proposed items are in their proposed crate;
- arrows: 267 of 267 proposed arrows exist; none missing; none extra;
- rules: no kind check fails; no break of the proposal's rules.

The plan's "Temporary breaks" table has one row, and it is CI's: macOS is
out of CI again for want of GitHub runners (#501).

Tests SHOULD NOT turn on `real` (Sam, 2026-11-12). A test that must is a
documented exception, listed with its reason in
`.github/scripts/real-exceptions.json`, which check-crates.sh reads: today,
only the fake product's tests, which drive it over HTTP with
kkt-agent-client, so its arrow to kkt-agent-client is a test arrow.

Every other difference was made on purpose during the refactor and charted
into the proposal, so the proposed map shows it. The plan's "Differences
from the proposed map" table lists each one, with its step, its reason and
whether Sam has accepted it.

Then Sam took phase 13 (2026-11-12, msg 3164), revising his phase 12
ruling the same day. The contract audit (#516) found that six of phase 12's
contract crates have one implementation and one or two consumers. In Rust
a consumer-declared trait always costs a forwarding adapter, so each pair
becomes one crate named for the scope: the contract and its stand-in in the
open, and the tool behind `real`, which only programs turn on. The six are
container, remote-access, browser, lane-commit, probe and agent-client;
kkt-client-command merges too. kkt-machine's `Machine` trait and its unused
stand-in go, and `Env` and `Platform` stay. kkt-service keeps its contract
crate, with launchd and systemd as two implementations. The rule's wording
for the project is in CLAUDE.md; Sam will settle the principles' own text
separately. A merged crate is a new kind, "contract and effect", which
kinds.py tells from "mixed": its effect is its own contract's one
implementation, and only programs turn its `real` on.

Phase 13 is done (steps 13.0 to 13.10). The plan's phase 13 section has a
table of what each step did against the audit and the plan, with every
difference and why. The final scorecard, on the code at 46911a8:

- crates: 65 of 65 proposed crates exist; none missing; none to remove;
- items: 793 of 793 proposed items are in their proposed crate;
- arrows: 256 of 256 proposed arrows exist; none missing; none extra;
- rules: no kind check fails; no break of the proposal's rules.

## Other decisions

- kkt-format is dissolved: the formatter becomes kkt-mcp-formatter.
  NoticeWording first moved to kkt-names with plain input (C5); msg 2148, the
  audit's F9, supersedes that. kkt-rules provides the default wording,
  DefaultWording, as its own implementation of its own OperatorWording. The
  program passes it in, and another product may pass its own, so kkt-server's
  Wording adapter goes.
- **The name kkt-rules stays** (msg 2149, the audit's F12). It is the
  implementation of kkt-api-agent's, kkt-api-operator's and kkt-api-admin's
  contracts, as its concern says. A deliberate exception to template 2.3.2's
  naming, which is a SHOULD: it is the application itself, not one
  interchangeable implementation.
- **Shutdown and the kkt-events bus stay concrete types** (msg 2150, the
  audit's F10). A pure in-memory coordination value, with no side effects or
  configuration, and which every implementation would make the same, may be
  passed as a concrete type. StopSignals stays a plain type in
  kkt-shutdown-os, which the program calls.
- random_chars is a provided method of Entropy, so kkt-entropy is a plain
  contract crate (msg 2011).
- kkt-agent-client stops depending on kkt-watch; the program's thin adapter
  maps one failure type to the other.
- Operator and admin names stay in kkt-names; tying an admin name to its key's
  hash moves to kkt-admission.

## The program's library (phase 10)

- **The library splits by who uses each part** (msgs 2789, 2794, 2803
  and 2825): kkt-operate (the operator's commands) and kkt-agent-commands
  (the agents' commands), composition crates beside kkt-server. Each takes
  its effects in a `Parts` value. No assembly depends on another or on an
  `-os` crate; kashkeshet chooses every implementation, keeps the adapters
  and `cli.rs` (the flags are the program's), and keeps a long dependency
  list on purpose, since it shows every choice in one place.
- **No kkt-serve** (msg 2876). The serving wiring stays in kashkeshet:
  choosing the database, reading the keys, resolving the settings,
  starting the rules and listening. kkt-server is already the server's
  assembly, and composing a server from the implementations it chooses is
  what the program is for. As a crate, kkt-serve would have one user,
  since kkt-keymode must not use it. So kashkeshet composes the server and
  the two assemblies of tools. (Phase 10 first planned kkt-serve as a third
  assembly; its step, 10.1, is dropped.)
- **The local server's facts are questions, not values.** Each assembly
  declares the small question its commands ask, and kashkeshet answers it
  from its own serving code or with kkt-operate's functions. Today each
  fact is read only by the command that needs it, and a missing record or
  a bad config file is reported only then, with its own text; a value
  worked out up front would change that. The one fact both assemblies ask,
  how a client reaches the server, is kkt-setup's `ConnectionSource`,
  which neither copies.
- **`Parts` takes the contract** (msg 2837): kkt-server's `Parts` takes an
  `Arc<dyn AdminApi>`, and `new_server_run` belongs to whoever calls
  `start`, so kkt-server no longer depends on kkt-rules in its code.
- **kkt-service's `Manager::for_this_os` takes plain paths** (msg 2829),
  the home folder and the config home folder, so kkt-service no longer
  depends on kkt-machine.

## Accepted assumptions

- SeededEntropy (repeatable randomness for the preview and the fake product's
  test, never for tokens) is a test double, so it sits in kkt-entropy behind
  `testing`.
- DryRun records changes and passes reads through, with no effects of its own,
  so it stays in kkt-service.
- kkt-search sits in foundations; Excerpt stays in kkt-db; the operator flag is
  part of the records.

## Rejected options

- Letting an implementation implement its one consumer's contract directly
  (msg 1952): it couples the two.
- Treating any adapter that decides nothing as a composition, so that kkt-store
  and kkt-server could implement other crates' contracts: kkt-store decides
  things, and kkt-server's case was ruled on directly instead (msgs 2015 and
  2019).
- Keeping the store's methods public on a concrete Store in kkt-rules (msg
  2026): the database stays private.
- record_run as an operation of the API: it would need the Rules it helps
  build.
- A shared "machine" contract for the six `-os` crates: each consumer declares
  its own needs (msg 2020).
- One kkt-api crate, or kkt-api-admin alone: the levels of authority are kept
  apart (msg 2045).
