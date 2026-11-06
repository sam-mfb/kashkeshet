# Could multi-tenant kkt be a separate project?

A study, 2026-11-06, by kkt-split-study for Sam, asked through sage-cardinal in #kkt-dev. No code changes. It reads the plan on this branch (`roadmap.md`, `agent-credentials-design.md`) against the code on `origin/next` at `0a03ade` (0.9.0 plus #366), which is where the 27 crates live; this branch's own tree predates the split.

The question: could multi-tenant kkt be a separate project that depends on kkt or on some of its crates, either as they are or after a restructuring? One example given: a shared `kkt-lib` repository that both `kkt` and a new `kkt-pro` depend on.

## Summary

**Not on the crates as they are.** The multi-tenant plan is not a layer beside kkt's core; it changes what the core's identities are. A swarm is a new input to the one visibility rule, `access::decide`. Users, orgs and roles are new rows that every scoped read filters by. The principal that admission names has to travel through `kkt-server`, `kkt-mcp` and `kkt-web` to the rules on every call. None of that can be added from outside, because the places that would have to change are private by design: `decide` is `pub(super)`, the store's SQL and migration list are closed, `Rules` has no trait in front of it, the tool set and its text are fixed in `kkt-mcp`, and `Admission` and `Principal` are closed enums. A separate project today would have to fork six crates (`kkt-rules`, `kkt-store`, `kkt-admission`, `kkt-mcp`, `kkt-web`, `kkt-server`), plus `kkt-events`, `kkt-format` and `kkt-config`, which carry the types those six exchange. That is the server. What it could reuse unchanged is the machinery around the server: the clock, names, shutdown, logs, backups, the machine-side crates (setup, container, service, client config, diagnostics, remote access, lane files) and the two client crates. Useful, but not where the work is.

**Of the four restructurings, the one that fits is (d): one repository, with login mode as a server setting in the one `kkt` binary.** That is what the roadmap already specifies (8.7: "login mode is a server setting, and a server is in one mode"), and the roadmap already draws the extension points of (b) where they are real: `kkt-admission`'s mode contract (8.9), `kkt-web`'s `Source` and `Gate`, and the three new crates, each behind a contract that `kkt-server` adapts. The core itself cannot be made pluggable without first teaching it the nouns, and once it knows them there is nothing left to plug in but login.

**The option worth keeping open is (c), a shared core, but only after step 2 lands.** Step 2 (roles, the principal carried to the rules) is the first point at which "the core" and "a product on top of it" are different things. Before it, there is nothing to separate; after it, `kkt-oidc`, `kkt-avatar-auth`, the login-mode pages and a second assembly could live in a second workspace or a second repository with the core unchanged in key mode. Even then the schema, the visibility rule, roles and the scoped reads stay in the shared core, with key mode using one default value for each noun, exactly as section 11 of the roadmap already plans.

**What would change the recommendation:** a licensing or ownership reason to keep multi-tenant code out of the open repository (then (c), after step 2, accepting that the nouns are in the open core); a decision that the key-mode product must never carry user, org or swarm columns (then only a fork works, and existing installs cannot move from `kkt` to `kkt-pro` without a one-way conversion tool); or a choice of sidecar for the authorization server (which shrinks `kkt-avatar-auth` to glue and removes the one argument for a second binary, the size of the login stack in the `kkt` binary).

The rest of this document gives the evidence: what the plan changes in each crate (1), where a separate project breaks (2), the four options compared (3), the hard parts (4), and the recommendation with its conditions (5).

## 1. What the plan changes in today's crates

The plan's steps 1 to 6 (roadmap, section 12) and its map of where the work lands (12a) touch the crates below. "Core" means the change alters what existing code does or the types it exchanges, so that today's callers and tests change. "Beside" means it adds new items next to existing ones and today's code path is untouched.

| Crate | What changes | Core or beside |
|---|---|---|
| `kkt-store` | New tables: users, roles, orgs, swarms, groups, grants, pending approvals, the actor record. New columns on agents and channels: internal ID, org, swarm, active flag. Every reference to an avatar moves from the handle to the internal ID, including the full-text index and foreign keys (step 1). `ChannelKind` and `ChannelScope` gain the swarm fact. "Storage queries by the capability's scope before counting or paging, never fetching wider and filtering" (section 5), so most `Reads` methods gain a scope parameter. New migrations, one per step at least, numbered from 0022 on `next`. | Core: the schema, the record types and the `Reads`/`Writes` signatures. |
| `kkt-rules` | `access::decide` takes the swarm as an input and `Who` gains the human capabilities. A `roles` module with `allows` and three scope-bound capabilities replaces every "is this the operator" check. `Actor` grows. The send limiter and open waits (`rate.rs`, `waiting.rs`) are keyed by internal ID. Waits and feeds decide visibility per event and end when the subscriber loses the right; today a wait authenticates once and ignores every notice that is not a message. `agent_view` splits details from memory. The notice sender becomes `server`. Grants, their cascade, deactivation, human changes through one avatar, the actor record. | Core: the rule, the actor types, waiting, and the public result types. Deactivation and grants are beside `hide_agent` and the stale cleanup. |
| `kkt-admission` | `Principal` names who was admitted (a user, not just "key holder"). The mode contract of 8.9 with three modes; the OpenID Connect mode stated in admission's own terms; the 401 challenge and resource metadata. | `Principal` and `Admission` are core types that every endpoint matches on. The modes are beside each other by design. |
| `kkt-web` | `Gate` returns a viewer and covers a redirect sign-in. `Source` becomes per caller; today one `Arc<dyn Source>` serves every viewer. `AgentView` and `Overview` split. Existing pages scope what they show by viewer. New pages: consent, device approval, user, org admin, server admin, swarms, groups. | Core: the contract and every existing page. The new pages are beside. |
| `kkt-mcp` | Both endpoints act for one caller instead of one shared `Rules`. Every `/mcp` call is bound to the grant's avatar in login mode, which changes the shared `me()` path. `register` and `whoami` gain two fields; `create_avatar` is new; admin `create_channel` gains `swarm`; a third endpoint `/mcp-user` with its own tool text and snapshot. | Core: the per-call identity. Tools and fields are beside, under the contract rule (add, never remove). |
| `kkt-server` | `admit` keeps the principal; `RulesSource` built per caller; `kkt-oidc` and `kkt-avatar-auth` adapted to admission and the rules; new endpoints mounted. | This crate is composition, so changing it is expected. It is where two products would differ most. |
| `kkt-events` | Notices carry the internal ID; the lag report carries no count. | Core: the notice types. |
| `kkt-format` | New fields; never names the personal swarm. Depends on `kkt-rules`' result types, so it changes when they do. | Mostly beside, pulled along by the rules' types. |
| `kkt-config`, `kkt-settings`, `kkt-provenance` | Login settings; a dozen new `Limits` fields (grant and token lifetimes, approval and unauthenticated-request rates, per-user sends and avatars), recorded by `settings_text`. | Beside: new fields on `Limits`. But `Limits` is one struct, so the fields must be added in this crate. |
| `kkt-hosts` | The public name beside the allowed `Host` names. | Beside. |
| `kkt-agent-client`, `kkt-watch`, the program's `credentials.rs` | The device grant; token files that hold refreshable tokens, refreshed through the injected server. Today's files keep loading. | Beside. |
| `kkt-setup`, the program's `cli.rs` | The bootstrap line; `--org` on `kkt lanes` and `kkt agent`. | Beside. |
| `kkt-names`, `kkt-lane-files` | Unchanged if the internal ID is a row ID; a lane file names no org. | None. |
| `kkt-clock`, `kkt-shutdown`, `kkt-logs`, `kkt-backup`, `kkt-container`, `kkt-service`, `kkt-remote-access`, `kkt-client-config`, `kkt-diagnostics`, `kkt-scratch` | Nothing, or a line (`kkt connect` writes a keyless entry in login mode). | None. |
| New: `kkt-oidc`, `kkt-avatar-auth`, `kkt-mock-issuer` | The OpenID Connect client, kkt's own authorization server for avatars, the test issuer. Each behind a contract adapted in `kkt-server`. | Beside, by construction. |

By size, the crates that change at their core (`kkt-rules`, `kkt-web`, `kkt-store`, `kkt-mcp`, `kkt-server`, `kkt-admission`, `kkt-events`, `kkt-format`, `kkt-config`) are about 1.26 MB of the 2.17 MB under `crates/`, and they are the server. The crates that do not change are the machine side and the plumbing.

The pattern across the table: what the plan adds beside existing code is login (the three new crates, the device grant, the settings). What it changes at the core is identity and scope (internal IDs, users, swarms, roles, the principal, per-caller rules and pages). The second kind is the multi-tenant part. The first kind is already drawn as separable.

## 2. Could a separate project build on the crates as they are?

No. Here is where it breaks, crate by crate, naming the decision made inside the crate that an outside project cannot change.

**The visibility rule is private.** `access::decide`, and `Channel`, `Who`, `Need`, `Decision` and `scope` with it, are `pub(super)` in `kkt-rules`. Every rule method calls it internally. An outside project cannot give it a swarm input, and cannot wrap it, because the callers are inside the same crate. Since #366, the store's channel filter is also derived from `decide` over the 16 `ChannelKind`s; `ChannelKind` is a plain struct in `kkt-store`, so adding a fifth fact changes every place that constructs one. The rule exists once, which is right, and that one place is inside `kkt-rules`.

**The store is closed.** `Reads` and `Writes` are fixed traits with a fixed SQLite implementation; only `sqlite.rs` holds SQL, and the one raw-SQL door (`execute_for_tests`) is behind the `testing` feature. `MIGRATIONS` is a private embedded list applied by `user_version`. An outside project cannot add a table, a column, a scoped query or a migration. It could open the same file with its own connection and run its own SQL, which breaks "only `sqlite.rs` has SQL", bypasses the store's write lock and reader pool, and collides on `user_version` with every later kkt migration (section 4).

**`Rules` has one implementation and no trait,** by rule (CLAUDE.md: "there is no trait in front of `Rules`"). `kkt-mcp`'s `endpoint` takes `Rules` by value; `kkt-server`'s `RulesSource` wraps `Rules`. An outside project cannot substitute scoped rules under the existing tools or pages. It could write its own `Source` implementation, which is a real extension point (the preview is the second implementation), but `Source` has no viewer today: `overview(&self)` answers the same for everyone, and `web::router` takes one `Arc<dyn Source>`. The per-viewer contract is step 2's change, inside `kkt-web`.

**The tool set is fixed.** The tools are methods on a private `ChatServer` under rmcp's `tool_router` macro, their text comes from `agent-tools.toml` and `admin-tools.toml` through `kkt-mcp`'s own `build.rs`, and `describe` fills the limits in. An outside project cannot add `create_avatar`, bind calls to a grant, or change what `register` does in login mode. It could mount a second endpoint with its own rmcp server (as `/mcp-user` will be), but not change `/mcp`.

**Admission is a closed enum.** `Admission` has two variants, `Open` and `SharedKey`; `Principal` has `Anyone` and `KeyHolder`. The crate's own doc says an OAuth validator "would be another variant": another edit, not a plug-in. Section 8.9 of the roadmap turns this into a mode contract, which is the right extension point, and it does not exist yet.

**The in-memory state is keyed by handle.** Open waits (`waiting.rs`) and the send limiter are inside `Rules`, keyed by handle, and `wait_for_messages` authenticates once. Per-event visibility and ending a wait on revocation need the subscriber's identity inside that state. Outside code cannot reach it.

**`Limits` is one struct.** The login-mode limits are fields of `Limits` by rule, checked by `Limits::check` and recorded by `settings_text`, with the build failing until they are. Fields cannot be added from another crate.

**The crates are not published.** Every crate has `publish = false`, `version.workspace = true` and path dependencies. A separate project would pin them as git dependencies at a commit or tag. That works, and it means every change to a core crate's public surface is a break for the dependent, with no semantic version to say so.

What an outside project could build today, honestly: a different `kkt-server` (its own assembly), its own web pages merged into the router, its own extra MCP endpoint, a login front door in front of `/mcp` that maps a bearer token to an access key. That is a proxy with a different front page, not multi-tenancy. The hard lines of section 2 of the roadmap ("every read is still checked at run time") cannot be enforced from outside the place that checks reads.

## 3. The restructuring options

Each option is judged on: what moves where, which crates are shared, what each project owns, versioning and release coupling, how existing installs keep working (design principle 3), the testing burden, and the effect on the rules in CLAUDE.md and PRINCIPLES.md.

### (a) kkt stays as is; kkt-pro forks it

- **What moves.** Everything is copied. kkt-pro changes the nine core crates in place and adds the three new ones.
- **Shared.** Nothing, in the sense that matters: the same code exists twice and drifts.
- **Ownership.** Each repository owns all of it. Every fix on `next` is a merge into the fork; `next` took 486 commits in the five weeks to 2026-11-06, many in `kkt-rules`, `kkt-web` and `kkt-store`, where the fork's changes are.
- **Versioning.** Independent, which is the appeal. The cost is a standing merge queue.
- **Existing installs.** Each fork keeps its own promise. But a database that has run under one fork cannot run under the other once both have added a migration: both write `user_version` 22 with different schema. So a kkt user can never upgrade to kkt-pro, and a pro user can never return, without a conversion tool. That is a new kind of break, and a permanent one.
- **Testing.** Every suite is duplicated. The contract snapshots diverge. The migration tests must cover both forks from the common ancestor and then from each fork's own history.
- **Rules.** CLAUDE.md and PRINCIPLES.md apply unchanged within each repository. The "one place" rules (`decide`, SQL only in `sqlite.rs`, one `Limits`) now have two places in the world, and PRINCIPLES' "contracts SHOULD NOT be broken" is broken in spirit: two versions of every contract, with the same names.

Cheapest on day one, most expensive every day after. The only case for it is a decision that the two products must never share a schema; see section 5.

### (b) kkt exposes extension points so kkt-pro plugs in, kkt unchanged for key mode

What would have to become a contract, and whether it can:

- **Admission mode.** Yes, and the roadmap already does it (8.9). This is a real and cheap extension point: `kkt-oidc` supplies a mode through `kkt-admission`'s contract; `kkt-server` connects them.
- **The authorization server.** Yes, already drawn: `kkt-avatar-auth` states what it needs in a contract that `kkt-server` adapts to the rules and store.
- **Pages.** Partly. New pages can be merged into the router from another crate. The existing pages must change to scope by viewer, which is inside `kkt-web`.
- **The visibility rule.** No. A swarm is a fact the rule reads, stored with the channel and the avatar. To let a plug-in add it, the core would have to store it and pass it to `decide`, which is the plan's own step 4a. A rule that accepts "extra facts" from a plug-in and lets the plug-in decide is a trait in front of the permission decision, which CLAUDE.md forbids and section 2 of the roadmap forbids more strongly: the hard line must be enforced in the one place that enforces lines, and that place must be in the product that promises it.
- **The store.** No. Rust traits cannot be extended from outside. A pro store would be a second store with its own SQL and its own migrations, which is the fork's schema problem again.
- **The tools.** Only by a tool-provider contract in `kkt-mcp` with tool text supplied by the product and a snapshot per product. Possible, and it moves the contract rule ("do not rename or remove") to a surface agents would then see differ by server. The plan avoids this by keeping one tool list and changing behavior by mode.
- **The principal, scoped reads, per-caller rules and pages.** These are the core changes of steps 1 and 2, and they must be made in the core for any plug-in to have something to scope.

So (b) collapses: to make kkt pluggable enough for multi-tenancy, kkt's core must first learn users, swarms, roles and the principal, with key mode using one default value for each. Once it has, the only things left to plug in are login's three crates and the login-mode pages, which the roadmap already puts behind contracts. (b) is the plan, at the granularity the plan already chose. The one thing (b) would add beyond the plan, a trait in front of `Rules`, is the one thing not to add.

- **Rules.** Adds contracts to `kkt-admission`, `kkt-avatar-auth` and `kkt-mcp`; CLAUDE.md's "no trait in front of `Rules`" stands. PRINCIPLES rule 2 (depend on what you need) is what makes the real extension points work.

### (c) A shared core repository, with kkt and kkt-pro as two thin products on top

- **What moves.** The crates move to the core repository. The question is which, and the answer from section 1 is: everything that knows the nouns, which is the whole server plus the plumbing, which is all 27 existing crates. What is left for a product to own: the binary (`src/bin/kkt`, `src/machine`, `src/commands`, `src/terminal`), its assembly, `tests/` and `tests/common`, the Dockerfile, Compose files, dist configuration and CHANGELOG. kkt-pro owns the same list plus `kkt-oidc`, `kkt-avatar-auth`, the login-mode pages (which must be in `kkt-web` unless `kkt-web` gains a way to take pages from the product) and `kkt-mock-issuer`.
- **Shared.** The core. In key mode it carries users, orgs, swarms and roles with default values, as section 11 of the roadmap already plans for the one binary.
- **Ownership.** The core owns every step of the plan from 1 to 5, since each is a change to the rules, the store or the contracts. The products own packaging. "Thin" is accurate for kkt and thin-plus-login for kkt-pro.
- **Versioning.** Three release trains: the core, kkt, kkt-pro. The core's crates must gain real versions (today `version.workspace = true`, `publish = false`) and be consumed by tag or from a private registry. Every step of the plan is a core release followed by two product releases. PRINCIPLES rule 9 ("most tasks stay in one crate") becomes "most tasks stay in one repository", which steps 1 to 3 already fail within one workspace and would fail harder across three.
- **Existing installs.** Better than the fork: one schema, one migration list, owned by the core, so a kkt database opens under kkt-pro and the reverse (the login tables are empty in key mode). The setup record, keys, service files and client configs are the product's and stay as they are. The migration tests belong with the core (the schema) but today run against the product's binary in `tests/`, so they split: the store's own tests for the schema, the product's for the binary.
- **Testing.** The core's unit and crate tests stay with it. The end-to-end tests in `tests/` need a binary, so each product runs its own copy against its binary, and the core cannot run them at all without a test program of its own. CI's per-crate feature check (`check-crates.sh`) runs in the core's workspace and must also run in each product's. The contract snapshot is per product.
- **Rules.** CLAUDE.md splits in three. The crate table moves to the core; each product keeps a short one for its binary. "Only `sqlite.rs` has SQL" and "every permission decision in `kkt-rules`" hold, in the core. The axum rule names a fourth crate in kkt-pro's repository, so the core's rule must name a crate it does not contain.

The structure is sound; the cost is in release coupling and the split test suite, and the benefit over (d) is only that kkt's binary does not carry the login stack and the pro code can be elsewhere. Those are reasons when the code must be elsewhere for licensing or ownership. They are not reasons of design.

### (d) One repository, with multi-tenant behind features or a separate binary

Three shapes, from best to worst:

- **(d1) One binary, login mode a server setting.** The roadmap's own shape (8.7, 12). The three new crates are workspace members; the `kkt` binary turns on their `real` features like every other crate's; `kkt serve` reads the mode from the config file and a server is in one mode. Key mode is unchanged throughout (section 11), and the one contract snapshot for `/mcp` holds for both modes, with `/mcp-user` pinned separately. Cost: the `kkt` binary carries an OpenID Connect client, a JWT verifier and an OAuth authorization server (or a sidecar's glue) that key-mode users never run. Much of the TLS stack is there already through reqwest and rmcp.
- **(d3) A second binary in the same workspace.** `kkt` and `kkt-pro` as two root packages sharing every crate; only `kkt-pro` depends on `kkt-oidc` and `kkt-avatar-auth`. This is (c) without the repository boundary: one migration list, one CI, one `check-crates.sh`, one PRINCIPLES. It doubles the packaging: two dist configurations, two installers, two images, `kkt setup`, `kkt upgrade` and `kkt doctor` aware of two products, two CHANGELOG sections. It buys a smaller `kkt` binary and a clear line for a reader: "pro is this package". It is the right first move if a product split is wanted, because it can later become (c) by moving the second root package and the three crates out, with the core's contracts already settled.
- **(d2) Cargo features.** `--features login` on the root package, with `#[cfg]` in the rules, the store or the web pages. Do not do this. Cargo features are meant to be additive and get unified across a build; a feature that changes what `decide` does, what the schema is or what the tools are is non-additive, every crate with `#[cfg(feature = "login")]` needs both builds checked, `check-crates.sh` gains a dimension, and the migration tests must run under both. It is the fork's duplication inside one tree.

- **Existing installs.** In (d1) and (d3), as the roadmap already plans: one schema, key mode unchanged, the three listed visible changes made once in step 1.
- **Testing.** One workspace, one suite, one CI. (d3) runs the end-to-end tests twice or parameterizes `tests/common` by binary.
- **Rules.** (d1) is what the roadmap's CLAUDE.md changes already describe (12a). (d3) adds a second program row to CLAUDE.md's program table and a rule for which crates only `kkt-pro` may depend on.

### The options side by side

| | Shared core crates | Product owns | Release coupling | Principle 3 | Tests | Rules changed |
|---|---|---|---|---|---|---|
| (a) fork | none | everything, twice | none, and that is the problem | breaks between products for good | duplicated | duplicated |
| (b) plug-ins | all, with the nouns inside | login crates, pages | one train | as the plan | one suite | becomes the plan, or adds a trait over `Rules` |
| (c) core repo | all 27 existing crates | binary, packaging, tests; pro adds 3 crates | three trains | one schema; setup files per product | split: core units, product e2e ×2 | CLAUDE.md ×3 |
| (d1) one binary, mode setting | all, in one workspace | one product | one train | one schema, as planned | one suite | as 12a already says |
| (d3) two binaries, one workspace | all, in one workspace | two root packages | one train, two artifacts | one schema | one suite, e2e per binary | one program row added |
| (d2) features | all, with `#[cfg]` | one package, two builds | one train, two builds | two schemas to test | matrix | CI dimension added |

## 4. The hard parts

Whatever the option, these four do not get easier, and they decide where the line between products could ever be.

### 4.1 The permission rule: `access::decide` and `ChannelScope` (#366)

Since #366 the rule exists once and the store's filter is derived from it: `access::scope` calls `decide` on each of the 16 channel kinds (hidden, open, member, owner), `ChannelScope` carries the allowed kinds, and `sqlite.rs` turns them into a 16-bit mask over one subquery (`SCOPE_CHANNELS`) used by the six queries that cover many channels. The test `the_stores_channel_filters_agree_with_decide` checks every query against `decide` for an agent and the human with each need.

Swarms add a fifth stored fact, "the channel's swarm is the viewer's swarm", so there are 32 kinds, the mask grows, the subquery compares swarms, and the six queries and the agreement test grow with it. `Who` gains the human capabilities, since a user sees every channel in the swarms they have access to and nothing else. All of this crosses `kkt-rules` and `kkt-store` together, in one change, because the kind is a contract between them. PRINCIPLES rule 9 tolerates that for a contract change; nothing outside the two crates can do it.

Two consequences for a product split. First, whichever product owns `decide` owns every hard line; a product that does not own it cannot add one, so the swarm fact must be in the shared rule, present in key mode too, where every channel is in the one personal swarm and the fact is always true. That gives a clean check for principle 3: in key mode the 32-kind mask with the swarm fact true must equal today's 16-kind mask, kind for kind. Second, the rule must stay `pub(super)`. The temptation under (b) or (c) is to make `decide` public so the product can call it; the product then has a second place to decide, which is the bug #366 just removed.

### 4.2 One schema and one migration list for two products

The store applies a private, ordered list by `user_version`, and `pending_upgrade` and the backup before an upgrade read the same number. Two products appending migrations independently collide: each writes 22 with a different meaning, and a database that has run under one is refused, or worse, misread, by the other.

The ways out:

1. **One list, owned by the core, applied in full by both products.** Login tables exist empty in key mode. Simplest, and what the roadmap's section 11 already does for the one binary. Under (c) the list lives in the core repository and products never add migrations.
2. **Two mechanisms:** `user_version` for the core and a `pro_schema` table for the product's own tables. Doable, and it invents a second migration system, teaches `pending_upgrade` and `kkt backup` about both, breaks "one numbered file in `migrations/`", and leaves the two sets ordered only by convention.
3. **A reserved number range** per product. Fragile; a core migration that lands in the range breaks the ordering for every install that already passed it.

Take the first. It also means that a database is a kkt database whichever product opened it, and that the migration fixture (the populated 0.9.0 database, schema 0019 plus 0020 and 0021 on `next`) is tested once. It rules out the fork, whose two lists diverge for good.

### 4.3 The MCP tool contract when the two products' tools differ

The tools are a contract with every agent (CLAUDE.md: add, never rename or remove), pinned by `tests/contract/tools.json` and `admin-tools.json`, with the text built from two toml files by `kkt-mcp`'s `build.rs` and the check that every tool has text and every text a tool.

If two products offered different tool lists on `/mcp`, each would need its own toml files and its own snapshot, `kkt-mcp` would need to take tool text from the product, and an agent moving between a kkt server and a kkt-pro server would meet a different contract under the same name. The roadmap's answer is better: one tool list on `/mcp` in both modes; `register`, `reclaim` and `create_avatar` present in both and refused with a plain message where the mode does not allow them; `register` and `whoami` gain fields that are null in key mode; `agent-instructions.md` says what differs by mode. The contract test then runs once against a key-mode server, and step 3's login-mode tests check the refusals with the mock issuer. `/mcp-user` is a new endpoint with its own snapshot, so it adds a third file and changes nothing in the first two.

The one open point: `create_avatar` is "login mode only" in section 9. For one contract it should be listed in key mode and refused there, like `register` in login mode, not hidden. The roadmap should say so.

### 4.4 The per-call identity path

Step 2 moves who the caller is from admission to the rules on every request: `Principal` in `kkt-admission`, kept by `kkt-server`'s `admit`, passed on by `kkt-mcp`'s endpoints and `kkt-web`'s `Gate`, acted on by `RulesSource` and `AdminServer` built per caller. Five crates, one change, and it is what makes everything after it possible: per-caller pages, scoped reads, waits that know who is listening. No product line can be drawn across this path; it must be finished in whatever holds the core before any product can be built on it. That is why section 5 says "after step 2".

## 5. Recommendation, and what would change it

**Do (d1): one repository, one binary, login mode a server setting, as the roadmap stands.** The core changes of steps 1, 2 and 4a (internal IDs, users, roles, the principal, the swarm fact in `decide`) are changes to what the core is, and they belong in the one place that owns the hard lines. The parts that are genuinely beside the core (login's three crates, the device grant, the settings) are already drawn behind contracts in separate crates. That is the real content of option (b), and it is already in the plan.

**Keep (c) available, and do nothing for it now.** The moment it becomes possible is after step 2, when the principal travels to the rules and the pages are per caller. If a reason appears then, the move is: lift `kkt-oidc`, `kkt-avatar-auth`, `kkt-mock-issuer` and a second root package into their own workspace or repository, leave every other crate in the core, and give the core's crates versions. Doing it earlier moves code that is about to change in every step.

**What would change this:**

- **Licensing or ownership.** If the multi-tenant product must be closed source, separately licensed, or owned by another team, then (c) after step 2, through (d3) first. Be clear about what stays open: the schema with users, orgs, swarms and grants, the visibility rule with its swarm input, roles and `allows`, the scoped reads, and the per-caller pages are all in the core. What the closed product adds is login (the OpenID Connect client, the authorization server or its sidecar glue), the login-mode pages, the device grant, and the assembly. If that is too little to be worth a separate product, the answer is (d1) with a commercial line drawn in configuration and support, not in code.
- **A decision that kkt's key-mode product must never carry the nouns.** Then no shared-core option works, because the nouns are in the store and the rule. Only the fork does, and it costs what section 3(a) says, above all that a database never moves between the products. A one-way `kkt-pro import` from a kkt database would be the mitigation, and it would have to be maintained against every kkt migration.
- **The authorization server as a sidecar** (Hydra or the like, roadmap section 13). Then `kkt-avatar-auth` is glue, the login stack in the `kkt` binary is the OpenID Connect client alone, and the one practical argument for a second binary (d3) disappears. (d1) becomes clearly right.
- **A change to the plan's shape.** If the plan were revised so that key mode and login mode did not share one schema and one `decide` (section 14 rejects that: "key mode and login mode on one server" is not the issue; "one server one mode" is kept), the analysis above would have to be redone. As the plan stands, its own section 11 is the strongest evidence for (d1): the migration gives every existing row a default org and swarm, and key mode never looks at them.

Two small things for the roadmap, found on the way: say that `create_avatar` is listed in key mode and refused there, for one contract (4.3); and note that the swarm fact in `decide` has a direct principle-3 check, that the key-mode mask equals today's (4.1).
