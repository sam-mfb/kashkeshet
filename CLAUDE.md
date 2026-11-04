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

Each directory or file under `src/` owns one area, described in the comment
at its top. Put new code in the part that owns its area. If none fits, add a
part named for the area (e.g. `search`), not for a kind of code (`types`,
`utils`).

| Part | Owns |
|---|---|
| `rules/` | Every permission decision and every change, for agents and the human |
| `crates/kkt-store/` | The `Reads`/`Writes` contract and its SQLite implementation |
| `format/` | Turning rule results into text or JSON |
| `mcp.rs`, `mcp_admin.rs`, `mcp_tools.rs`, `mcp_transport.rs`, `tool_text.rs` | The agent and admin MCP tools; what the two share; the endpoints' HTTP transport; their text |
| `web/` | The human's pages, through its own contract (`web/source.rs`) |
| `crates/kkt-admission/` | Whether an HTTP caller is let in at all; web sessions |
| `crates/kkt-hosts/` | Which host names the server answers to |
| `app.rs`, `web_real.rs` | Assembling the server from its parts; the server's adapters to the web contract |
| `main.rs` | The `kkt` command: subcommands and the files and environment they read |
| `crates/kkt-config/` | Every limit, fixed setting and default, and their checks |
| `crates/kkt-settings/` | Gathering a server's settings from flags, config file and defaults |
| `crates/kkt-provenance/` | What produced a server run, and the build script that records the git commit |
| `crates/kkt-clock/`, `crates/kkt-names/` | Time (`Clock`); handles, IDs, randomness (`Entropy`) and the name packs |
| `crates/kkt-events/`, `crates/kkt-shutdown/`, `logs.rs` | New-message notifications; the stop signal; the server's log file |
| `setup/`, `docker/`, `service/`, `crates/kkt-remote-access/` | `kkt setup`; running kkt in Docker; as a service; over Tailscale |
| `clients/`, `diagnostics.rs` | Client MCP configs; `kkt doctor`'s checks |
| `crates/kkt-backup/`, `crates/kkt-lane-files/` | Backup folders; lanes as files |
| `watch/`, `mcp_client.rs` | `kkt watch`; the shared MCP client |

Outside `src/`: `crates/kkt-store/migrations/` (the schema, one numbered file per change);
`agent-instructions.md` and `admin-instructions.md` (what agents are told
when they connect); `agent-tools.toml` and `admin-tools.toml` (what agents
are told about each tool); `build.rs` (builds the tool text); `tests/contract/` (the tools agents see, pinned);
`CHANGELOG.md` (see `docs/releases.md`).

## Rules

**Storage.** Only `crates/kkt-store/src/sqlite.rs` imports rusqlite or contains SQL.
Rules ask for what they need through named methods on `Reads` or `Writes`.
A rule that must be atomic runs as one `Db::write`. Schema changes go in a
new numbered file in `crates/kkt-store/migrations/`. Migrations only add; never edit one that
has shipped.

**Permissions.** Every check on whether an agent or the human may use a
channel goes through `rules::access::decide`. To an agent outside a closed
channel, the channel does not exist: the answer is `not_found`, exactly as
for a missing ID.

**The human.** The human operator is an actor in `rules`, beside agents. A
change the human makes goes through the same rule as an agent's, and a
change to a channel posts a message in it from the operator saying what
changed, in the same `Db::write`.

**What agents see.** The MCP tools are a contract with every agent. Do not
rename or remove tools, parameters, error codes or output fields; add new
ones. Tool and parameter text lives in `agent-tools.toml` and
`admin-tools.toml`, never in Rust, with limits written as placeholders such
as `{max_limit}`. After an intended change, run
`UPDATE_CONTRACT=1 cargo test --test contract` and review the diff to
`tests/contract/`. Keep `agent-instructions.md` in step with the tools.

**kkt's names for the principles.**
- Time comes only from the injected `Clock`, randomness only from `Entropy`.
- Dependencies are injected through `rules::Deps` or `app::Parts`. There is
  no trait in front of `Rules`; it has one implementation.
- Every limit and default lives in `crates/kkt-config/src/lib.rs`, is checked at startup, and
  appears in `provenance::settings_text`. Bad settings stop the server with
  an error naming the setting.
- Rules return `RuleError` with a fixed code from `rules/error.rs`. Logging
  happens only at the edges (`main.rs`, `mcp.rs`, `mcp_admin.rs`, `web/`). `unwrap` and
  `expect` are for tests only.
- Stored rows that record activity carry the `run_id` of the server run that
  made them.
- Third-party types stay inside the part that wraps them.

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
- Unit tests sit beside their code; rule tests in `src/rules/tests.rs` use
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
