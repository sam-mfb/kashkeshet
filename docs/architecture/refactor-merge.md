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
map's "Current" view the code again.

The workspace went from 33 packages to 60. The code now matches the
proposed map, as amended during the refactor, except for two differences
recorded with their reasons in `decisions.md` ("After the refactor").

## Final scorecard

From `progress.py`, on the code at the end of the refactor:

- crates: 60 of 60 proposed crates exist; none missing; none to remove;
- items: 797 of 797 proposed items are in their proposed crate;
- arrows: 222 of 223 proposed arrows exist; 1 missing (kkt-server →
  kkt-api-admin); none extra;
- rules: no kind check fails; 1 break of the proposal's rules (kkt-rules'
  tests use kkt-db-sqlite).

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

The macOS checks are back in the Rust checks' matrix since #486, on
`refactor` itself, so the merge's pull request need not touch them.

The jobs the refactor added stay: the crate outside the workspace that
builds against the shared crates by git (`outside-crate`), and the check
that the programs do no machine work (`programs-do-no-machine-work`).

## What to watch on macOS

The macOS checks were off for most of the refactor, so its macOS code was
reviewed by hand only until #486 turned them back on. Watch their first
runs. The code the refactor touched that decides by platform:

- `crates/kkt-machine-os/src/lib.rs`: `OsMachine::platform` says whether
  this build is macOS or follows XDG. It replaces `Platform::CURRENT` in
  kkt-settings' `paths.rs`, which is gone. Every path built from `Platform`
  (the config folder, the data folder, the default database) rests on it.
- `crates/kkt-terminal-os/src/browser.rs`: the program that opens the
  Tailscale login link (`open` on macOS, `xdg-open` elsewhere), moved from
  the program's `src/terminal/browser_login.rs`, with its test.
- `crates/kkt-service/src/lib.rs`: `Manager::for_this_os` chooses launchd
  on macOS and systemd on Linux. The choice is unchanged, but the values
  it reads changed: the home folder and the config base now come
  from kkt-machine's `Env`, and launchd's wait and retry times from
  kkt-service's `Settings`.

Code under `cfg(unix)` also moved, and runs on both systems: file modes
(kkt-admission-os, kkt-backup-os, kkt-client-config-os, kkt-container-os,
kkt-config-file-os), the stop signals (kkt-shutdown-os), the agent
credential files' owner and mode checks (kkt-agent-client), and the closed
output pipe (kkt-terminal-os).

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
