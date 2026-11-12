# Contract crate audit

An independent look at every crate that `docs/architecture/concerns.json`
marks as a contract, on `refactor` at 5d2f0c8. For each one it asks whether
it is needed, or whether the consumer could declare what it needs in its
own terms and have the implementation supplied to it (ARCHITECTURE.md 3.1
and 3.3). It reports only; nothing in the code is changed.

There are 17 contract crates. Nine are clearly worth keeping. Six have one
real implementation and one or two consumers, and would be simpler merged
with their implementation. One is unclear, and one should stay but has a
trait nobody consumes.

| Crate | Verdict |
|---|---|
| kkt-api-agent | keep |
| kkt-api-operator | keep |
| kkt-api-admin | keep |
| kkt-db | keep |
| kkt-identity | keep |
| kkt-clock | keep |
| kkt-entropy | keep |
| kkt-log | keep |
| kkt-service | keep |
| kkt-machine | keep the data; its `Machine` trait is not needed |
| kkt-container | merge with kkt-container-docker |
| kkt-remote-access | merge with kkt-remote-access-tailscale |
| kkt-browser | merge with kkt-browser-os |
| kkt-lane-commit | merge with kkt-lane-commit-git |
| kkt-probe | merge with kkt-probe-reqwest |
| kkt-agent-client | merge with kkt-agent-client-rmcp |
| kkt-client-command | unclear; leans keep |

## How to read this

**Counting.** A *consumer* is a crate whose code calls the trait or uses the
contract's records. The program, which only builds the implementation and
hands it in, is not counted. A crate that only passes the dependency on to
another crate (as kkt-operate passes the prober to kkt-diagnostics) is
counted, but marked as passing it on. Stand-ins are counted as stand-ins,
and I say whether any other crate's tests use them.

**What removal looks like in Rust.** Rust has no structural typing: a type
satisfies a trait only through an `impl` someone writes, and the orphan rule
says that `impl` must sit in the crate that owns the trait or the crate that
owns the type. So when a contract crate goes, there are four shapes, and each
entry below says which one applies.

- **A. Program adapter.** The consumer declares the trait. The program wraps
  the tool's type in a new type of its own and writes an `impl` that forwards
  each method. This is the forwarding adapter Sam's ruling of 2026-11-12
  removed. kkt-lane-files' `Disk` already works this way (`LaneDisk` in
  `src/commands/lanes.rs`).
- **B. Wrapper depends on its consumer.** The consumer declares the trait,
  and the tool's crate depends on the consumer to implement it. ARCHITECTURE
  3.1.2.3 forbids this, and it would also put an effect crate on top of an
  assembly or logic crate. It is listed only to rule it out.
- **C. Merge.** The contract and its one wrapper become one crate named for
  the area. The trait, records and stand-in are always compiled. The tool
  is behind `real`. Consumers depend on the crate without `real`; the
  program turns `real` on and injects the tool directly. There is no
  adapter, and no consumer can name the tool, because CI builds each crate
  alone (`check-crates.sh`) and the tool's type does not exist there. The
  cost is that the crate is two kinds at once (contract and effect,
  ARCHITECTURE 4), and its stand-in lives in the implementer (3.2.4).
- **D. A function.** For a contract with one method, the consumer takes a
  function value instead of a trait, and the program passes the tool's
  function. `OperateParts::try_bind` already works this way. There is no
  trait, no adapter and no contract crate.

## Each crate

### kkt-api-agent — keep

1. Declares `Api` and its areas (`Agents`, `Channels`, `Messages`,
   `Memory`, `Lanes`), with every record and error shared by the levels.
   kkt-rules implements it, and kkt-api-sim implements it again over a
   made-up world. Nine crates consume it: kkt-identity,
   kkt-identity-keymode, kkt-api-operator, kkt-api-admin, kkt-mcp,
   kkt-mcp-formatter, kkt-operate, kkt-server and kkt-web, plus three
   programs.
2. A second implementation exists, kkt-api-sim, and a real program uses it
   (kkt-web-preview). It is test support, not a release.
3. Removal: each surface (kkt-mcp, kkt-web) would declare its own trait.
   kkt-rules cannot depend on them (3.1.2.3, and it would invert the
   layers), so the program would need adapters forwarding dozens of methods
   for each surface.
4. Lost: the one place that defines records and error codes for every
   surface and for kkt-pro. Gained: nothing worth having.
5. **Keep.** Many consumers and two implementations (3.3.3, 3.3.4).

### kkt-api-operator — keep

1. Declares `OperatorApi` (`Views`, `Changes`, `Lanes`, `Server`) and
   `Notices`. kkt-rules and kkt-api-sim implement it. Six consumers:
   kkt-api-admin, kkt-mcp, kkt-mcp-formatter, kkt-operate, kkt-server and
   kkt-web.
2. Second implementation: kkt-api-sim, as above.
3. Removal: as for kkt-api-agent, with kkt-web declaring the trait and
   the program forwarding every method.
4. Lost and gained: as for kkt-api-agent.
5. **Keep.** 3.3.3 and 3.3.4.

### kkt-api-admin — keep

1. Declares `AdminApi` (`Admin`, `Swarms`) and `AdminChannel`, 109 lines.
   Only kkt-rules implements it. Consumers: kkt-mcp, kkt-mcp-formatter,
   kkt-server and kkt-fake-product.
2. No second implementation, and none likely in kkt. kkt-pro may consume it.
3. Removal: kkt-mcp would declare the trait, and kkt-rules cannot implement
   it there (3.1.2.3), so the program would forward every method. It could
   instead fold into kkt-api-operator, which it already depends on. That is
   a different question from loose coupling, and saves one small crate.
4. Lost: one definition for four consumers. Gained: little.
5. **Keep.** More than two consumers (3.3.4), among them the fake product,
   which stands in for another product.

### kkt-db — keep

1. Declares the database contract: values, tables, the query and change
   language, transactions, search, migrations, `Backend` and `Evolve`.
   kkt-db-sqlite implements it; so do `MemoryPlace`, `FastPlace` and
   `FaultyBackend` behind `testing`. One consumer, kkt-rules' store; the
   program uses only `Upgrade`.
2. Three working implementations besides SQLite, all stand-ins, and a
   shared conformance suite every backend must pass. The docs anticipate
   Postgres (`Backend`'s write rule mentions a Postgres advisory lock).
3. Removal: kkt-rules would declare the language and kkt-db-sqlite would
   depend on kkt-rules to implement it, which is 3.1.2.3. Adapter A is not
   realistic for a query language.
4. Lost: the conformance suite, the fast stand-in for the rules' tests, and
   the line that keeps SQL out of the rules (7.2). Gained: nothing.
5. **Keep.** One consumer, but several implementations and a shared test
   suite (3.3.3, 7.2).

### kkt-identity — keep

1. Declares `Identify`: turning an admitted caller into a swarm and an
   actor. Implemented by kkt-identity-keymode and by kkt-fake-product's
   `FakeIdentify`, and expected from kkt-pro. Consumers: kkt-mcp and
   kkt-server.
2. Two real implementations today, in two programs. The `FixedIdentify`
   stand-in is used by no other crate; kkt-mcp's and kkt-server's tests
   write their own.
3. Removal: kkt-server would declare the trait, and every product's identity
   crate would depend on kkt-server or need a program adapter.
4. Lost: the one place products plug in who a caller is (CLAUDE.md's "No
   user in the shared crates" rests on it). Gained: nothing.
5. **Keep.** Several implementations across products (3.3.3, 1.3).

### kkt-clock — keep

1. Declares `Clock`, and owns `Time`, `Span` and time formatting (the only
   chrono user). kkt-clock-system implements it, with `FixedClock` behind
   `testing`. About twenty consumers.
2. `FixedClock` is used by fifteen crates' tests.
3. Removal is not realistic: every consumer would declare its own clock.
4. Lost: one time type and one stand-in for the whole workspace.
5. **Keep.** 3.3.4 by a wide margin.

### kkt-entropy — keep

1. Declares `Entropy`. kkt-entropy-os implements it; `SeqEntropy`,
   `SameHandleEntropy` and `SeededEntropy` are stand-ins behind `testing`.
   Seven consumers: kkt-admission, kkt-names, kkt-rules, kkt-server,
   kkt-web, kkt-api-sim and kkt-fake-product.
2. The stand-ins are used by nine crates' tests.
3. Removal: as for the clock.
4. Lost: shared deterministic randomness for tests.
5. **Keep.** 3.3.4.

### kkt-log — keep

1. Declares `Log`, with levels and fields. kkt-logs-os' `TracingLog`
   implements it; `CapturedLog` is the stand-in. Consumers: kkt-mcp,
   kkt-web, kkt-server and kkt-web-preview.
2. `CapturedLog` is used by kkt-web, kkt-server and kkt-server-listen.
3. Removal: each consumer would declare its own log trait and the program
   would forward three times.
4. Lost: one log contract and its shared stand-in.
5. **Keep.** 3.3.4, and RUST.md 10 needs logic crates to log through
   something injected.

### kkt-service — keep

1. Declares `Service` (install, start, stop, status, logs, uninstall) and
   also owns the plan of what the service runs and `check_bind`. Implemented
   by kkt-service-launchd and kkt-service-systemd, chosen by platform, and
   by kkt-service's own `Unsupported`; `Memory` is the stand-in. One
   consumer, kkt-operate.
2. Two real implementations on two platforms. The stand-in is used by
   kkt-operate's tests.
3. Removal: kkt-operate would declare the trait, and the program would need
   two forwarding adapters of twelve methods each, or the wrappers would
   depend on kkt-operate (3.1.2.3).
4. Lost: one definition for two implementations. Gained: nothing.
5. **Keep.** Multiple implementations exist (3.3.3). The plan logic it
   holds sits uneasily with 3.2.5.1, but that is about what is in the crate,
   not whether it should exist.

### kkt-machine — keep the data; drop the trait

1. Declares `Env` and `Platform` (plain facts about the machine), and the
   `Machine` trait that gives them. kkt-machine-os implements `Machine`;
   `FixedMachine` is the stand-in. The data is used by five crates
   (kkt-agent-commands, kkt-client-config, kkt-setup, kkt-container-docker,
   kkt-operate) and the program. The trait is called only in
   `src/bin/kkt/main.rs`.
2. No second implementation. `FixedMachine` is used nowhere.
3. Removal of the trait: kkt-machine-os gives `env()` and `platform()` as
   plain functions and the program calls them, as it calls other effect
   crates. No adapter. The data types still need a shared home, so the
   crate stays.
4. Lost: an unused stand-in. Gained: one less trait. No consumer is
   injected with a `Machine`, so nothing is decoupled by it.
5. **Keep the crate, drop the trait.** The data has five consumers (3.3.4);
   the trait has none but the program, which is composition and may call an
   effect crate directly (4.3.1).

### kkt-container — merge with kkt-container-docker

1. Declares `Container` (16 methods), its records (`Folder`, `State`,
   `Login`, `KeyCopy` and others), Docker's file names and an exit code. It
   also declares `ConfigReader` and `LoginLink`, which are *Docker's* needs:
   kkt-container-docker calls them and the program implements them.
   kkt-container-docker implements `Container`; `Memory` is the stand-in.
   Consumers: kkt-operate (the trait) and kkt-setup (records and file names
   only).
2. No second implementation, and none likely: a Podman setup would run the
   same Compose files. The stand-in is used by kkt-operate's tests.
3. Removal, shape C: one `kkt-container` crate with the trait, records,
   `ConfigReader`, `LoginLink` and `Memory` always compiled, and `Docker`
   behind `real`. kkt-operate and kkt-setup depend on it without `real`.
   The program turns on `real` and passes `Docker` as it does now. No
   adapter, no new dependency on effect code. Shape A instead would mean a
   16-method forwarding adapter, and kkt-setup would still need the records
   from somewhere below kkt-operate, since kkt-operate depends on kkt-setup.
4. Lost: CLAUDE.md's rule that a contract crate has no `real` would not
   apply, because the crate would be marked an effect crate. Gained: one
   fewer crate. And `ConfigReader` and `LoginLink` would sit with the code
   that needs them (3.1.1). The "contract" already holds names like
   `docker-web-key`, so it is in practice Docker's own surface.
5. **Merge.** Two consumers and one likely implementation (3.3.2). The
   crate's own contents show it is shaped by its implementation, not by
   its consumer.

### kkt-remote-access — merge with kkt-remote-access-tailscale

1. Declares `RemoteAccess` (prepare, serve, served, stop) and `Node`.
   kkt-remote-access-tailscale implements it; `Memory` is the stand-in. One
   consumer, kkt-operate.
2. No second implementation. The stand-in is used nowhere: kkt-operate's
   tests write their own fake.
3. Removal, shape C: one `kkt-remote-access` crate, with `Tailscale` behind
   `real`. Shape A would be a four-method adapter in the program.
4. Lost: a separate crate that holds no logic and whose stand-in nobody
   uses. Gained: one fewer crate.
5. **Merge.** One consumer, one implementation, no likely second: the case
   3.3.2 describes.

### kkt-browser — merge with kkt-browser-os

1. Declares `Browser` (open a link, report a failure later). kkt-browser-os'
   `SystemBrowser` implements it; `Recorder` is the stand-in. One consumer,
   kkt-container-docker, which is itself an effect crate.
2. No second implementation. `Recorder` is used nowhere: kkt-container-docker's
   tests write their own.
3. Removal, shape C: one `kkt-browser` crate with `SystemBrowser` behind
   `real`. kkt-container-docker depends on it without `real`. Shape A would
   be a two-method adapter.
4. Lost: nothing used. Gained: one fewer crate.
5. **Merge.** 3.3.2.

### kkt-lane-commit — merge with kkt-lane-commit-git

1. Declares `LaneCommit`, one method: the commit a folder is at.
   kkt-lane-commit-git's `Git` implements it; `Memory` is the stand-in.
   Consumers: kkt-lane-files (calls it) and kkt-operate (passes it on).
2. No second implementation likely. The stand-in is used by kkt-lane-files'
   tests.
3. Removal: shape C, one `kkt-lane-commit` crate with `Git` behind `real`;
   or shape D, kkt-lane-files takes a function and the program passes
   `Git`'s. Note that kkt-lane-files already declares its other dependency,
   `Disk`, itself and the program adapts kkt-lane-files-os to it (shape A).
   One consumer is treated two ways.
4. Lost: the shared `Memory`, which is a few lines. Gained: one fewer crate.
5. **Merge.** One real consumer and one implementation (3.3.2).

### kkt-probe — merge with kkt-probe-reqwest

1. Declares `Prober` (probe a server; read a key file) and its records
   `KeyFile` and `Check`. kkt-probe-reqwest's `HttpProber` implements it;
   `Memory` is the stand-in. Consumers: kkt-diagnostics (calls it) and
   kkt-operate (passes it on, and builds `Check` values).
2. No second implementation likely. The stand-in is used nowhere:
   kkt-diagnostics' tests write their own.
3. Removal, shape C: one `kkt-probe` crate with `HttpProber` behind `real`.
4. Lost: nothing used. Gained: one fewer crate. Separately, `Prober` joins
   two unrelated jobs, an HTTP probe and reading a file's mode, which is a
   1.1 question whatever happens to the crate.
5. **Merge.** One real consumer and one implementation (3.3.2).

### kkt-agent-client — merge with kkt-agent-client-rmcp

1. Declares `Servers`, `Avatars` and `Watched`, with their records and
   `Failure`. kkt-agent-client-rmcp implements them over MCP; `Memory` is
   the stand-in. Consumers: kkt-agent-commands (`Servers`) and kkt-watch
   (`Watched`).
2. No second implementation likely; another transport is not planned. The
   stand-in is used nowhere: kkt-agent-commands and kkt-watch each write
   their own fakes.
3. Removal, shape C: one `kkt-agent-client` crate with the MCP client
   behind `real` (rmcp and reqwest stay optional). Shape A is poor here:
   `Servers` returns `Box<dyn Watched>`, so kkt-agent-commands' trait would
   have to name kkt-watch's, and the program would forward three traits.
4. Lost: CLAUDE.md's rule that kkt-agent-commands does not depend on an
   implementation would need rewording to "not with `real`". Gained: one
   fewer crate, and a stand-in that nobody uses could go.
5. **Merge.** Two consumers and one implementation (3.3.2). This is a
   weaker case than the others because its surface is large, but the
   principles point the same way.

### kkt-client-command — unclear; leans keep

1. Declares `ClientCommand` (remove kkt's server from a client, add it).
   kkt-client-command-claude's `Claude` implements it; `Recorder` is the
   stand-in. Consumers: kkt-client-config (calls it); kkt-operate and
   kkt-agent-commands pass it on.
2. One implementation today. A second is plausible, since other agent
   clients also configure MCP servers through their own command line. The
   stand-in is used by two crates' tests.
3. Removal, shape C: one `kkt-client-command` crate with `Claude` behind
   `real`. Shape A would be a two-method adapter, written twice (for
   kkt-operate's parts and kkt-agent-commands').
4. Lost: a stand-in two crates use; and if a second client arrives, the
   crate comes back. Gained: one fewer crate.
5. **Unclear.** It depends on whether more clients are expected (3.3.3).
   If they are, keep; if Claude Code stays the only one, merge.

## Where the principles leave room

- **Naming pulls two ways.** 1.2 says to name a module for its scope unless
  several modules implement it. 2.3.2 says a module implementing an
  outside contract is named for it (`vpn-tailscale`). With one
  implementation, 1.2 alone would already merge most of the pairs above.
  CLAUDE.md's rule of 2026-11-12 (a contract crate and a wrapper for every
  tool) chose 2.3.2. Adopting the merges above means revising that rule,
  with check 6 of `check-crates.sh` and the kinds in `concerns.json`.
- **A merged crate breaks other rules.** Shape C is not named in the
  principles. It makes a crate two kinds (4), and puts the stand-in in the
  implementer (3.2.4). Shape A keeps those rules but brings back the
  forwarding adapters. The principles treat an adapter as cheap
  (3.1.2, 4.5.1). In Rust it is a new type and a forwarding method for each
  trait method, because of the orphan rule. The principles do not weigh
  that cost.
- **Who counts as a consumer.** 3.3.2 and 3.3.4 count modules that depend on
  another. They do not say whether a crate that only passes a dependency on
  counts. kkt-operate passes the prober, the lane commit and the client
  command to other crates; counting it raises each to two consumers.
- **Unused stand-ins.** 2.5 and 3.2.4 ask contract modules for stand-ins.
  Six contract crates' stand-ins (kkt-remote-access, kkt-browser,
  kkt-probe, kkt-agent-client, kkt-identity, kkt-machine) are used by no
  other crate; each consumer writes its own fake. The principles do not
  say whether a stand-in nobody uses is a reason to keep a contract. Here
  I treated it as no reason.
- **What a contract may hold.** 3.2.5.1 forbids implementation-specific
  methods and configured values in a contract module. kkt-container holds
  Docker's file names and an exit code; kkt-service holds the plan of what
  the service runs. The line between "values used by the contract" (3.2.5)
  and implementation detail is not drawn.
- **Single-method contracts.** The principles speak only of modules and
  adapters. In Rust a function value (shape D) is a lighter form of
  injection that needs no crate and no adapter. kkt already uses it for
  `try_bind`. The principles neither encourage nor rule it out.

## Effect on existing installs

None. This is a report; it changes no code. The merges it recommends would
move code between crates and change no file, flag or tool an install
relies on.
