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

- **Every crate has a kind**: contract, logic, effect, composition,
  configuration or test support, plus mixed in the current map only. A
  contract may hold pure behaviour on its own value types (Time arithmetic,
  query checks), but no effect and no wiring. Only programs and compositions
  depend on effect crates.
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
- **The operator flag is part of the records**, filled by kkt-rules, which
  makes the operator an actor and names it.
- **Limits.** The limits the rules enforce come from the agent level's
  `limits`; the page's own (messages per page, excerpt length) are kkt-web's.
  The default name pack is kkt-settings', so kkt-container needs no API crate.

## The web page and identity

- **Web sessions move into kkt-web** (msg 2021). kkt-admission keeps the
  access decision; the gate asks it only whether a key is good and who a
  viewer is. A session holds the gate's own value, since kkt-web cannot name
  kkt-admission's Principal.
- **The preview counts as a program**, like the fake product, so it implements
  the contracts directly.
- **Identity, one contract per consumer** (msg 2025). kkt-mcp declares
  Identify, and kkt-server declares WebIdentify, the one question its web gate
  asks. The program adapts the same KeyMode to each.

## kkt-config

- **kkt-config is broken up by domain** (msg 1979). Format and fixed facts go
  to the crate that owns them; tuning values become settings the program
  fills; kkt-config keeps only what is truly cross-domain. The page's
  kkt-config table lists every item.

## Other decisions

- kkt-format is dissolved: the formatter becomes kkt-mcp-formatter, and
  NoticeWording moves to kkt-names with plain input.
- random_chars is a provided method of Entropy, so kkt-entropy is a plain
  contract crate (msg 2011).
- kkt-agent-client stops depending on kkt-watch; the program's thin adapter
  maps one failure type to the other.
- Operator and admin names stay in kkt-names; tying an admin name to its key's
  hash moves to kkt-admission.

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
