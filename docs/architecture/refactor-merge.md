# Merge `refactor` into `next`

This is the description for the pull request that merges the `refactor`
branch into `next`. Sam opens it.

## What the refactor did

It reorganized the workspace's crates so that every dependency is
injected and each crate is one kind: a contract (traits and plain values),
logic with every effect injected, an effect on the machine, or the
assembly. It changed how the code is divided, not what kkt does. The plan
is `docs/architecture/refactor-plan.md`, the decisions behind it are in
`decisions.md`, and the page in `docs/architecture/` shows both the code
("Current") and the plan ("Proposed").

It ran in phases, each step one pull request into `refactor` with the full
checks passing. Phase 1 laid foundations: hashing, the clock, randomness,
the stop signal, listening and search each got a crate of their own, and
kkt-format became kkt-mcp-formatter. Phase 2 moved each operating-system
implementation into an `-os` crate, which the program adapts to its
crate's contract. Phase 3 removed kkt-config, giving each item to the crate
that uses it, and split the config file, the machine's facts and the
settings into their own crates. Phase 4 split logging into its rules, its
files and a shared log contract. Phase 5 made kkt-db a contract only,
merged kkt-store into kkt-rules and made the store private, and put the
application's operations behind three contract crates: kkt-api-agent,
kkt-api-operator and kkt-api-admin. Phase 6 moved the web sessions into
kkt-web, made the page use the kkt-api traits directly, and made the page
preview a program of its own. Phase 7 moved the last machine work out of
the programs. Phase 8 made CI check two of the new rules and made the
map's "Current" view the code again. Phase 9 removed kkt-settings: each
owner checks and words its own settings, and kkt-server takes them checked.
Phase 10 moved the program's tools out of its library into two assemblies,
kkt-operate for the operator's commands and kkt-agent-commands for the
agents', while the program keeps composing the server. Phase 11 gave kkt-db
a fast test stand-in, so kkt-rules' tests use no effect crate, and the
program's tests run the store and the rules on the real backend. Phase 12
split every third-party dependency into a contract crate in kkt's terms and
a wrapper named for the tool that implements it directly, which the program
injects with no adapter: Tailscale, Docker, launchd and systemd, the MCP
client, git, the doctor's HTTP probe, Claude Code's command and the
browser opener. Phase 13 merged each of those pairs that has one
implementation back into one crate named for the scope, of a new kind,
"contract and effect": the contract in the open, and the implementation
behind `real`, which only programs turn on. That is container,
remote-access, browser, lane-commit, probe, agent-client and
client-command; kkt-service keeps its contract crate, with launchd and
systemd as two implementations. kkt-machine's unused `Machine` trait went.
The map draws a merged crate as a split box, and its grey and amber arrows
now run from the crate that is used or built to the crate that uses or
builds it.

The workspace went from 33 packages to 65. The code now matches the
proposed map, as amended during the refactor, with no difference left
(`decisions.md`, "After the refactor").

## Final scorecard

From `progress.py`, on the code at the end of the refactor (46911a8):

- crates: 65 of 65 proposed crates exist; none missing; none to remove;
- items: 793 of 793 proposed items are in their proposed crate;
- arrows: 256 of 256 proposed arrows exist; none missing; none extra;
- wiring: 102 of 102 proposed wirings exist; none missing; none extra
  (counted since the proposal's wiring was charted after 13.10; until then
  the proposal had 42 of the 95 in the code's record, and 21 that record
  lacked, 7 of them the web preview's, which the record now has);
- rules: no kind check fails; no break of the proposal's rules.

## Effect on existing installs

None. The database, the setup record, the Docker folder and its Compose
files, service files, keys and key files, the config file and its settings,
client configs, CLI commands and flags, log files and the MCP tools are as
they were. Every step's pull request said so and how it checked. The 0.10.0
migration test (`tests/migrations/v0_10_0.rs`) still answers every pinned
read and ends up the same as a new database, and the contract tests
(`tests/contract/`) still pin the tools agents see: no step changed them.

The shared crates' public surface does change, for crates that build on them
by git such as kkt-pro. CHANGELOG.md's Unreleased section says what a
dependant must change.

## What this pull request must change in CI

- Remove `refactor` from `.github/workflows/ci.yml`'s branch lists, under
  both `pull_request` and `push`.

The macOS checks are out of the Rust checks' matrix again since #501, for
want of GitHub runners, with a comment saying how to put them back. The
merge's pull request need not touch them; they come back by their own pull
request, or on a self-hosted runner.

The jobs the refactor added stay: the crate outside the workspace that
builds against the shared crates by git (`outside-crate`), and the check
that the programs do no machine work (`programs-do-no-machine-work`).

## What to watch on macOS

The macOS checks were off for most of the refactor. #486 turned them back
on for a while, and #501 turned them off again, so its macOS code is
reviewed by hand until they return. Watch their first runs. The code the refactor touched that decides by platform:

- `crates/kkt-machine-os/src/lib.rs`: `platform()` (`OsMachine::platform`
  until 13.8) says whether
  this build is macOS or follows XDG. It replaces `Platform::CURRENT` in
  kkt-settings' `paths.rs`, which is gone. Every path built from `Platform`
  (the config folder, the data folder, the default database) rests on it.
- `crates/kkt-browser/src/system.rs`: the program that opens the Tailscale
  login link (`open` on macOS, `xdg-open` elsewhere), moved from the
  program's `src/terminal/browser_login.rs` to kkt-terminal-os, then to its
  own wrapper (12.5d), then into kkt-browser behind `real` (13.3), with its
  tests.
- `src/machine/service.rs`: `for_this_os` chooses launchd on macOS and
  systemd on Linux, and injects kkt-service-launchd's `Launchd` or
  kkt-service-systemd's `Systemd` (12.3). The choice is unchanged, but the
  values it reads changed: the home folder and the config base now come
  from kkt-machine's `Env`, and launchd's wait and retry times from
  kkt-service-launchd's `Settings`.

Code under `cfg(unix)` also moved, and runs on both systems: file modes
(kkt-admission-os, kkt-backup-os, kkt-client-config-os,
kkt-container (behind `real`), kkt-config-file-os), the stop signals
(kkt-shutdown-os), the agent credential files' owner and mode checks
(kkt-agent-files-os), and the closed output pipe (kkt-terminal-os).

## How to merge

One pull request, merged with a merge commit, with no force-push, after a
final review of the whole diff and a full CI run on both systems ("What
needs Sam", item 6).

## What awaits Sam

`refactor-plan.md` begins with "Awaiting Sam": every difference from the
proposal Sam has not yet ruled on, and the open questions, each with a
recommended answer. They need not hold up the merge: each was made on
purpose, with its reason, and none affects an install.

## For the principles template

Sam wants the principles to say, for future projects, that settings are not
a domain: each component owns its own settings, their defaults and their
checks, and gathering them for a program is composition, not a component of
its own. kkt learned this twice, with kkt-config and kkt-settings (phase 9).
Sam is also considering a note for RUST.md on two points about tests. How
to handle integration tests: a suite written for its purpose, such as the
rules against the real database, not coverage hidden in an exception a later
developer may undo. And that a test stand-in is free to use any tool, the
real engine's included, since its job is to serve the tests, not to mimic
the real implementation.

Sam edits the template himself; this pull request does not change
`principles/`.

### Notes on third-party dependencies, for Sam's edit (nobody edits `principles/`)

From Sam's ruling of 2026-11-12: a third-party dependency is always split
into a contract crate named for the scope and a wrapper crate named for the
tool, which implements the contract directly. The program only injects it.
Phase 13 revised it the same day (note 6): a tool with one implementation
is one crate named for the scope, the implementation behind `real`. Notes
1 to 5 stand for a tool with several implementations, as kkt-service has.

1. **3.1.2.1** should say that the wrapping module implements the project's
   contract directly, and that a parent only injects it: no adapter between
   them.
2. **3.3.1** (loose coupling for third-party dependencies) conflicts with
   3.2.3.1's example, where `vpn-tailscale` implements the `vpn` contract
   that `network` depends on.
3. **Where contracts are defined** (section 2 or 3.2), it should say that a
   contract is in the project's terms, never the tool's: "run `tailscale`
   with these arguments" is not a contract.
4. **1.2.1 against 2.3.2** for a single implementation: 1.2.1 names the
   module for its scope, while 2.3.2 names an implementer of an external
   contract for the tool. Sam's ruling: always split, and name the wrapper
   for the tool.
5. **A wrapper crate is an effect crate that also holds the logic for
   driving its tool** (Sam's answer to the survey's question 3, msg 3050):
   kkt-container-docker held both Docker's knowledge and the code that
   runs `docker` (since 13.1, kkt-container's implementation behind
   `real`, which holds both still). That sits against principle 4 (a module is one kind) and
   4.5.2 (complex adapter logic as its own logic module). Sam thinks this
   may be worth addressing in the principles. His reason for the shape: the
   scope's contract is the abstraction other developers use, and only
   whoever works on the wrapper thinks about the tool.
6. **A consumer-declared dependency costs an adapter in Rust** (from the
   contract audit, #516). 3.1.1 has the consumer declare each dependency
   in its own terms, which assumes a language where a type meets an
   interface by its shape, as in Go. Rust has no such typing: a type meets
   a trait only through an `impl`, which must sit beside the trait or the
   type. So a consumer-declared trait always costs a forwarding adapter, a
   new type with one forwarding method per trait method, in a third crate.
   For a dependency with one implementation, the shape that fits Rust is
   one crate that holds the contract (the trait, its records and its
   stand-in) in the open and the implementation behind a feature, which
   only programs turn on. A separate contract crate fits a dependency with
   many consumers or several implementations. 3.2.4 (where stand-ins
   live), 3.3 (when to use a contract module) and 4 (one kind per module),
   and RUST.md, should say which shape applies when. This would revise
   Sam's ruling of 2026-11-12, which always splits a third-party
   dependency into a contract crate and a named wrapper.

### Candidates to revisit (not decided)

Sam wants these kept to one side, to come back to. None is a rule yet, and
nobody acts on them.

1. A component takes the value it needs, not a record it reads one field
   of. (From kkt-config-file and kkt-service taking plain paths in place of
   kkt-machine's `Env`.)
2. Duplication that is accidental is not a reason to abstract. (From kkt-db's
   test stand-in having its own SQL translation, apart from kkt-db-sqlite's.)
3. In Sam's words, not yet a precise rule: a module with one consumer that
   simply pulls out related functions is a smell. (From kkt-serve, which was
   dropped.)
