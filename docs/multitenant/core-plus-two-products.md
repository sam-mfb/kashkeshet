# Core plus two products

The follow-up to `project-split-study.md`, written 2026-11-06 by kkt-split-study from the answers given in #kkt-dev (messages 665, 666 and 671 to 673) to Sam's two follow-ups, with Sam's decisions of the same day as sage-cardinal relayed them. The study asked whether multi-tenant kkt could be a separate project; this document takes Sam's premises as fixed and says what a shared core would be, what it costs, and what is still open. Code facts are from `origin/next` at `0a03ade`.

## Summary

Sam's premises: kkt stays open source, keys only, one user on a local machine. kkt-pro is a commercial product for many users and orgs, at maximum security. The two have separate schemas and a one-way upgrade, free to pro. The goal is to avoid maintaining a fork.

The answer is a shared core whose central concept is the **swarm**: an ID that avatars and channels belong to, with every lookup bound to it. Key-mode kkt has exactly one swarm, with no owner. kkt-pro adds orgs, users, roles, groups and grants above the swarms, in its own tables keyed by the same swarm ID, and maps each request to an actor in one swarm before the core sees it. The core never learns what a user is.

The shape holds, and it improves on the roadmap on one point: `access::decide` and `ChannelScope` do not change at all. The swarm test the roadmap adds to `decide` becomes "every store lookup is bound to one swarm", so a channel in another swarm is missing before the rule is asked, and the 16 channel kinds stay 16. The price is that the core must know "swarm" as its own concept, so kkt's database carries a `swarms` table with one row; and that the org is half a core concept, because handles, lanes and the org announcement are per org, not per swarm. That half is the open "realm" question below.

Nearly all of today's server becomes core: by size, everything in the server crates but the assembly. What is key-only is the assembly and the `kkt` program. What is pro-only is new code, not copies. The fork is avoidable, and worth avoiding: since the crate split on 2026-11-03 the crates that changed most (kkt-rules 15 commits, kkt-mcp 11, kkt-web 8) are exactly the ones the core shares.

Sam decided on three repositories: `kkt-core` (public, every shared crate), `kashkeshet` (public, what it provides today on top of the core) and `kkt-pro` (private, the core plus everything new). Most changes will then take two or three pull requests: one in the core, a tag, then a bump in each product. kkt-pro's private CI builds against the core's latest `main` on a schedule so breakage shows early.

Getting there is an eight-step refactor of today's kkt into the core, each step shipping with key mode unchanged and the 0.9.0 database migrating. It replaces the roadmap's steps 1 and 2 and is about their size. Two things in the plan can be deferred: internal avatar IDs and the rename of the notice sender.

Still open, for Sam: one database file with a swarm column, or one file per swarm; and whether the realm (the core's view of an org, grouping swarms for lanes and handle uniqueness) is in the core, or lanes and handles are pro's through contracts.

`roadmap.md` is not changed by this document. It will be revised if Sam adopts this shape: its steps 1 and 2 are replaced by the refactor in section 6, steps 3 to 6 become kkt-pro's work on its own tables, and the swarm input to `decide` is dropped.

## 1. Premises and decisions

**Sam's premises.**

- kkt stays open source: keys only, local, one user.
- kkt-pro is a commercial product: many users and orgs, maximum security.
- Separate schemas, and a one-way upgrade from free to pro.
- Avoid the maintenance headache of a fork.

**Decided by Sam, 2026-11-06.**

1. **Three repositories.** `kkt-core`, public, holds every shared crate. `kashkeshet`, public, uses `kkt-core` to provide what kashkeshet provides today. `kkt-pro`, private, uses `kkt-core` plus everything new. Section 5 writes out what this means.
2. **Naming.** The core's concept is the swarm, not "space": an ID that avatars and channels belong to, with every lookup bound to it. Key-mode kkt has exactly one swarm, with no owner. kkt-pro keeps everything else about a swarm in its own tables, keyed by the same ID: owner, personal or shared, groups, name, deactivation. If the core needs a grouping above swarms, it is the **realm**: the core's view of an org, a grouping of swarms for lanes and handle uniqueness, and nothing else.

**Still open, for Sam.**

- One file with a swarm column, or one file per swarm (section 4).
- Realm in the core, or lanes and handles in pro through contracts (section 3, lanes and handles).

## 2. What can be shared, if the rules, store and schema were separate

This was the first follow-up: suppose key-mode kkt never contains users or swarms, and the core and the schema are separate per product. The answer is what shaped the rest: the identity-free parts are the plumbing, and everything that matters runs through an actor and a scope. Sizes are source bytes, tests included unless said.

**Shared unchanged** (about 530 KB): kkt-container (143), kkt-service (90), kkt-client-config (52), kkt-diagnostics (44), kkt-watch (38; everything it needs is injected, and token refresh belongs to the `Server` implementation), kkt-clock (33), kkt-names (32), kkt-lane-files (33), kkt-remote-access (22), kkt-backup (19), kkt-logs (11), kkt-hosts (10), kkt-shutdown (4), kkt-scratch (3).

**Shared after a small change.**

| Crate | KB | In the way | Change |
|---|---|---|---|
| kkt-events | 14 | notice fields are handles as strings; pro wants internal IDs | keep IDs as opaque strings, or add an `id` field. Who may see an event is decided in the rules, so the crate stays identity-free |
| kkt-config | 45 | `Limits` is one struct with 32 public fields; pro needs about 12 more | pro keeps a second struct in its own crate |
| kkt-settings | 45 | gathers exactly `Limits` | expose the parsed document, or a hook to gather another section |
| kkt-provenance | 31 | `settings_text` covers `Limits`, and the build fails on a missing field | take a second settings text from the product |
| kkt-admission | 36 | `Admission` and `Principal` are closed enums | the mode contract of roadmap 8.9; `Principal` becomes the mode's payload. The shared key, sessions and server keys (21 KB) are then identity-free |
| kkt-agent-client | 27 | `Client` holds a key-mode avatar token | the credential becomes a contract: present a header, refresh when told |
| kkt-setup | 268 | the question flow is per product | the Docker, service, client, switch and upgrade mechanics are shared; the questions are not. Medium, not small |

**The middle ground inside the five server crates**, if the rules and store were per product: about a tenth of their 1,110 KB is identity-free and worth pulling out. The `Db` wrapper and migration runner in kkt-store (about 10 KB); the error codes, token hashing and rate limiters in kkt-rules (14 KB, marginal); the HTTP transport, tool-text building and placeholder filling in kkt-mcp (18 KB); `Change`, sign-in cookies, toasts and the page shell in kkt-web (60 KB, with the snag that askama templates inherit within one crate's folder); the size cap in kkt-format (14 KB). Message storage, lanes, memory, waits, the tools and the pages are not identity-free: each takes an actor and a scope, filters by them, and shows the result. Lanes and memory are generic over an owner only in their content rules; each is a `Rules` method running in one `Db::write` over the store's traits.

So separate rules and stores would share plumbing and duplicate everything that changes. That is why the second follow-up asked about a shared core with the swarm inside it.

## 3. The swarm as the core's concept

Structurally, a swarm is one key-mode server's worth of avatars and channels: an avatar is in exactly one swarm, fixed at creation; a channel lives in one swarm; an avatar sees only its swarm's channels and avatars (roadmap 1, 3, 14). So the core runs one swarm's rules, key-mode kkt is the core with exactly one swarm and no humans above it but the operator, and kkt-pro adds many swarms plus orgs, users, roles, groups and grants above them, mapping each request to an actor in one swarm.

### What in the plan strains the picture, and how much

| Item | Fits? | What the core needs |
|---|---|---|
| A channel and an avatar in exactly one swarm; an avatar sees only its swarm | Exactly. This is today's server | A swarm ID on avatars and channels, and every read and write bound to one swarm |
| Lanes per org, not per swarm | Half. Lane content, versions, taking and import are swarm-free; the catalogue an avatar may take from is wider than its swarm | A realm that swarms point to, or lanes supplied by pro through a contract. Open |
| Handles unique per org; `handle_taken` across the org | Half. The core can make handles unique per swarm (today) or per realm | The realm again: uniqueness checked by the core within it, in the same transaction, so pro keeps no handle registry of its own. Open |
| Announcements: server, org, user | Half. The server announcement is the core's. The org and user ones are pro's, and the user one depends on who owns the avatar | The core takes extra announcement texts from the identity contract at connect, `register`, `reclaim` and `whoami`, shows them in order, and stores none |
| The operator or human as a member | Fits better than today. In pro a human changes a channel through one avatar (roadmap 5), a swarm actor. The human's read-everything view is today's `Actor::Human` | Core actors: `Avatar`, `Operator { words }` (sees all, changes with a notice; today's Human and named Admin), `Reader` (sees all, changes nothing; today's `UnnamedAdmin`). The words in a notice ("an org admin", "crisp-crane's user") come from pro as text; `operator_words` already builds them from the actor |
| A user's web view spans several swarms | Strains the core's pages, not the core. The core's `Source` becomes one swarm's view; pro composes across the swarms a user may see | Core web pages become single-swarm components; pro's pages are its own. The core's live feed is per swarm; pro merges |
| Revocation cascade across swarms | Pro's. The cascade walks pro's grants and calls the core's deactivate in each swarm | `deactivate(swarm, avatar, words)` in the core, atomic within the swarm, and `end_waits(swarm, avatar)` |
| Deactivation itself | Fits. Removal from channels, notices, ownerless channels, memory kept: all within one swarm | The operation beside `hide_agent`, as planned, with the words supplied |
| Swarm deactivation, owner, personal flag, name, groups | Pro's. The core's swarm has none of these | kkt-pro's swarm table keyed by the core's swarm ID |
| The admin endpoint | Fits per swarm. Today's admin tools are "operator in the swarm" | Core admin tools take a swarm; pro's admin endpoint picks the swarm and adds its org tools beside |
| Per-event visibility; feeds ending on revocation | Fits. Within a swarm it is today's rule; the subscriber's right ends when pro says so | A subscription per (swarm, actor) the core can end |
| The actor record inside the change's transaction (roadmap 5) | Needs a hook | The core's change methods accept an attached record (who, as text pro chooses, plus opaque data) stored in the same transaction. A general mechanism, not an audit framework |

Nothing structural fails. What is weakened: "the swarm is chosen at approval and fixed for good" is pro's rule, enforced by pro never passing another swarm, so the guard is a capability type (a swarm handle constructible only by pro's `allows`), not a core check. The one-file-per-swarm variant in section 4 removes even that.

### The core's interface

- **Swarm as a parameter.** Every `Rules` method takes a swarm, or `Rules::in_swarm(id)` returns a bound view. Every `Reads` and `Writes` method takes it, and every SQL statement filters by it. `ChannelScope` carries the swarm as a hard filter beside its kinds; `decide` is untouched.
- **Actors.** `Avatar(Agent)`, `Operator { words }`, `Reader`. Pro decides who is which in which swarm, from its roles and grants, before calling the core.
- **The identity contract at the edges.** kkt-mcp's endpoints take an `Identify` contract: a request becomes (swarm, actor, extras) or a refusal, where the extras are the announcements and the `whoami` additions (permissions, deadline, swarm name). Key mode: the token lookup, one swarm, no extras. Pro: the grant check. kkt-web's `Gate` returns the viewer the same way. `register` and `reclaim` stay tools; in pro `Identify` says they are refused and why, so the tool contract stays one.
- **Waits and feeds.** `wait_for_messages(swarm, avatar, ...)` as today, plus `end_waits` and a per-swarm subscription with a handle pro can close.
- **Where `decide` and `ChannelScope` live.** In the core, in kkt-rules and kkt-store, private as now. Pro never calls `decide`. Pro's own decision, who may be which actor in which swarm, is a different question in a different crate. That is the layering: pro chooses the actor, the core applies the rule within the swarm.
- **Transactions.** Pro's writes that must be atomic with a core change (a grant with the avatar it creates; the actor record) use the attached-record hook, or a shared connection (section 4).

### The realm, stated plainly

If the realm is in the core, it is the core's view of an org and nothing more: a grouping of swarms, used for two things, which lanes an avatar may take and within what set handles are unique. It has no users, no admins, no announcement of its own. Key mode has one realm with one swarm. Pro maps its org to the realm and keeps everything human about the org in its own tables.

If the realm is not in the core, lanes become a contract the core calls (pro supplies the catalogue), handles are unique per swarm in the core, and pro enforces org-wide uniqueness with its own registry before creating an avatar, outside the core's transaction. That is a larger change to the core's lane code, and a race pro must close itself. The realm is the smaller change. Open for Sam.

## 4. The schemas

Two ways to make the swarm real, and they differ a lot.

| | A swarm column, one file per server | One SQLite file per swarm |
|---|---|---|
| Core tables | `swarms` (id, realm, created_at, run_id) and a `swarm` column on agents and channels; messages and memberships follow their channel; lanes keyed by realm. Key mode: one swarm, one realm | Today's schema, unchanged. A swarm is a file; the core opens it as today |
| Pro tables | orgs, users, roles, groups, swarm facts (swarm id to owner, personal, active, org, name), grants, approvals, invitations, claim bindings. They reference `swarms.id` and `(swarm, handle)`. Same file or a second one | The same, in pro's own file, referencing swarm files by ID |
| `decide` | unchanged; the swarm is a hard filter in every query | unchanged; nothing to filter |
| Migrations | two named sets in one file: the core's keeps `user_version`, so existing installs upgrade exactly as today; pro's is tracked in a `schema_versions` table by the shared runner. Each product's SQL stays in its own store crate | the core's list for swarm files, pro's for its file |
| Joint transactions | possible when pro's tables are in the same file: a shared SQLite crate owns `Db` and the connection, each store crate implements its trait over it, and one `Db::write` holds a core and a pro write. rusqlite then lives in three crates | not across files. Approval becomes: write the grant pending, create the avatar, activate the grant. The plan's "consumed once, atomically" weakens to a two-step with a recovery rule |
| Cross-swarm reads: a user's web view, org admin pages, org-wide search and activity, the cascade | one query with `swarm IN (...)` where the core offers it, else a loop | always a loop with merging and paging across N sorted lists; N live subscriptions merged |
| Operations | one file, one pool, one backup | a file and a pool per swarm, a personal swarm per user, so thousands of files; pro needs an open-file cache and per-file backups |
| The hard line | a filter in every core query, tested once in the core; pro passing a swarm it was not granted is the bug, guarded by the capability type | by construction: nothing in a swarm file can name another swarm |
| One-way upgrade, free to pro | pro opens the kkt file: the core's migrations are already applied; pro's set creates its tables, one org, one user who owns the one swarm as a personal swarm | the kkt file becomes swarm 1's file; pro's file is new |

Separate schemas, in Sam's sense, holds for pro's tables, which kkt never has. The core's tables are shared because the core's code is; fully separate agents and channels tables would be the fork. Under the column design kkt's file carries a `swarms` table with one row and a `swarm` column; that is where the premises meet, and it is the price of sharing.

The author's lean: the column, for the first version, with the core's API per swarm so the file design stays open. Cross-swarm reads are most of what humans do in pro; joint transactions keep approval and the actor record atomic as the plan wants; one file is what backup, upgrade and `kkt doctor` already handle. The file design is the stronger line and the simpler upgrade, and the right answer if "maximum security" means the line must not depend on a filter being present in every query. Open for Sam.

## 5. Three repositories

Sam's decision, written out.

| Repository | Visibility | Holds |
|---|---|---|
| `kkt-core` | public | every shared crate (below), its own CI with `check-crates.sh`, PRINCIPLES.md and the crate table of CLAUDE.md, the schema tests and the 0.9.0 migration fixture |
| `kashkeshet` | public | the `kkt` program (`src/bin/kkt`, `src/machine`, `src/commands`, `src/terminal`), kkt-server (the key-mode assembly), `tests/` and `tests/common` against the `kkt` binary, the Dockerfile and Compose files, dist configuration, CHANGELOG, the contract snapshots |
| `kkt-pro` | private | the `kkt-pro` program, kkt-pro-server, and the new crates (below); its own tests, image, release process and snapshots |

**Which crates go where.** The boundary is enforced by the repositories: a core crate cannot depend on a product crate because it is not there to depend on, which is stronger than a rule in CLAUDE.md.

| kkt-core | kkt-pro | kashkeshet |
|---|---|---|
| kkt-rules (swarm-bound, actors as in section 3), kkt-store, a new shared SQLite crate (`Db`, pools, named migration sets), kkt-events, kkt-format, kkt-mcp (the agent and admin tools behind `Identify`), kkt-admission (the mode contract; the shared key stays here), kkt-web (single-swarm pages, the shell, `Change`), kkt-config, kkt-settings, kkt-provenance, kkt-hosts, kkt-clock, kkt-names, kkt-shutdown, kkt-logs, kkt-backup, kkt-watch, kkt-agent-client (credential as a contract), kkt-lane-files, kkt-diagnostics, kkt-scratch, and kkt-container, kkt-service, kkt-remote-access, kkt-client-config, kkt-setup, which both products' programs use | kkt-pro-rules (orgs, users, roles, groups, grants, transitions, `allows`), kkt-pro-store, kkt-pro-web (users, groups, swarms, grants, consent and approval pages; composition across swarms), kkt-pro-mcp (`/mcp-user`, the org admin tools), kkt-pro-server, kkt-oidc, kkt-avatar-auth, kkt-mock-issuer (dev only) | kkt-server, the `kkt` program |

By size, today's server crates are about 1,216 KB; all of it but kkt-server (56 KB) is core. The key-only side is the assembly and the program. Pro's side is new.

**Versioning and the flow of a change.**

- The core's crates get real versions (today every crate has `publish = false` and `version.workspace = true`). The core releases by tag; each product depends on the core by git tag, or by a private registry if that proves easier. Semantic versioning on the core: a change to a crate's public surface is at least a minor bump before 1.0.
- A change that touches the core takes **two or three pull requests**: one in `kkt-core`, then a tag, then a bump pull request in `kashkeshet` and, when the change concerns it, in `kkt-pro`. A change that stays in one product takes one. Most changes to channels, messages, waits, lanes, memory, tools or pages are core changes, so most changes take two or three. That is the cost of the boundary, and it is the price of not forking: each is made once.
- `kkt-pro`'s private CI builds against the core's latest `main` on a schedule, nightly say, as well as against its pinned tag on every pull request. A core change that breaks pro then shows within a day, before the core tags, instead of at the next bump.
- PRINCIPLES.md rule 9 ("most tasks stay in one crate") becomes, across repositories, "a core change lands in the core first, with its contract, and products follow". The roadmap's habit of splitting a cross-crate step into pull requests that each build on a merged contract carries over directly.

**Tests.** The core keeps every crate's unit tests, the store's schema tests and the migration fixture. Today's end-to-end tests in `tests/` start the `kkt` binary, so they stay in `kashkeshet`. The core needs its own way to run a server in tests: kkt-server's `listen` behind its `real` feature can start one in-process from a test, or the core keeps a small test program. Which is a decision for the move. `kkt-pro` runs its own end-to-end tests with the mock issuer. The contract snapshot for `/mcp` and `/mcp-admin` is checked in both products, since both serve the core's tools; `/mcp-user` is pro's alone.

**Releases.** Three trains. The core tags when a change is ready; `kashkeshet` releases as today, from a bump; `kkt-pro` releases privately. A kkt release that needs a core change is two tags apart.

## 6. The refactor, in eight steps

Today's kkt becomes the core in steps, each shipping with key mode unchanged and the 0.9.0 fixture migrating. The work happens in the kashkeshet repository first; the move to `kkt-core` comes when the shape is proven (step 8).

1. **The swarm.** A `swarms` table with one row, a `swarm` column on agents and channels, every SQL statement filtered by it, `ChannelScope` carrying it, `Rules` bound to a swarm through `Deps`. kkt-store and kkt-rules. The largest step: about 60 store methods and every rule method thread one value. `decide` and the agreement test do not change. Migration test from 0.9.0. None of the three visible changes of roadmap 11 is needed: the notice sender can stay `operator` in key mode.
2. **Actors.** `Human`, `Admin(name)` and `UnnamedAdmin` become `Operator { words }` and `Reader`, with the words built at the edges from what they are built from now. kkt-rules, kkt-mcp, kkt-server. Mostly a rename; notices unchanged.
3. **The identity contract.** `Identify` on kkt-mcp's endpoints, the viewer on kkt-web's `Gate`, `end_waits` and per-swarm subscriptions in kkt-rules. This is the roadmap's step 2 in another form, crossing the same five crates.
4. **The realm**, if Sam keeps it in the core: lanes and handle uniqueness keyed by realm; announcements beyond the server's taken from `Identify`. kkt-store, kkt-rules, kkt-mcp.
5. **The shared SQLite crate** with named migration sets; `user_version` kept for the core's set. kkt-store, one new crate.
6. **The attached change record.** One hook in the core's change methods. kkt-rules, kkt-store. Can wait until pro needs it.
7. **Web.** Pages take a swarm; the shell and `Change` made reusable. askama's `dirs` can point at another crate's templates folder within a workspace, so the shell need not be copied; across repositories it is copied or the core ships it as a file the product's build reads. kkt-web.
8. **A second program** that assembles key mode again from the same crates, to prove the shape before any pro crate exists; then the move of the shared crates to `kkt-core`.

**This replaces the roadmap's steps 1 and 2.** Both thread one identity through the same five crates; the difference is what is threaded. The plan threads the principal and capabilities, pro's concepts, into the core. This threads the swarm and the actor, the core's own concepts, and keeps users out. The cost is about the same. Two things from the plan are deferred: internal avatar IDs (`(swarm, handle)` is a fine key while handles are unique per realm, and nothing in pro's tables needs more) and the notice sender rename.

**Existing installs.** No visible change in any step. Each step's migration is tested from the previous step's schema and from 0.9.0. Setup, Docker, service files, keys, client configs and flags are untouched.

## 7. Maintenance, against a fork

| | Fork | Core plus two products |
|---|---|---|
| A change to channels, messages, waits, lanes, memory, formatting, tools, admin tools, pages | twice, or merged into the fork with conflicts wherever pro diverged | once, in the core; two or three pull requests |
| A change to users, orgs, roles, grants, login | pro only | pro only |
| A change to keys, setup, service, Docker | kkt only, or both if pro kept them | once, in the core |
| Schema | two lists; no path between products; a conversion tool maintained against every kkt migration | one core list plus pro's set; the upgrade is "open the file" |
| Tool contract | two `/mcp` snapshots that drift; agents meet two contracts under one name | one `/mcp` snapshot and one admin snapshot, checked in both products; pro adds `/mcp-user` |
| Tests | the rule tests (219 KB) and the page scenarios (58 KB) exist twice | once, in the core |
| Releases | two trains, nothing between them | three trains, coupled by tags |
| Discipline | none; everything drifts | every core change takes a swarm and an actor and must not learn a user; the core's tests enforce it once; the repository boundary enforces the rest |

Since the split, the crates that changed most are the core under this shape. The fork duplicates them; the core shares them.

## 8. Where Sam's instinct is right, and where it isn't

Right:

- Org, user and swarm are a layer above channels and avatars. The core needs no user, role or grant; `decide` does not change; lanes, memory, messages, waits and formatting are reusable as they are.
- The fork is avoidable and worth avoiding.
- The free product is not too simple to share. Nearly all of today's server is core; the key-only part is the assembly and the program.

Not quite:

- A layer on top needs a hook below. The core must know the swarm, with one in key mode, and kkt's file will carry that table and column. "Separate schemas" is true of pro's tables only.
- The org is half core, through the realm, or pro takes lanes and handles through contracts. Either way the core is touched by something org-shaped.
- The human side is pro's, and it is large: every user page and every org admin page composes across swarms, and the core's pages become single-swarm components. Expect kkt-pro's web crate to be as big as the core's.
- Under the column design, the swarm line is a filter in every core query, tested once in the core and guarded by pro's capability type. If "maximum security" means the line must not rest on that, take the file design and pay for it in cross-swarm loops and file handling.
- The one-way upgrade is cheap in both designs only because the core's tables are shared. It is the strongest argument for the shape and against the fork.

## 9. What the roadmap would change

`roadmap.md` is unchanged by this document. If Sam adopts the shape, it will be revised: steps 1 and 2 are replaced by the refactor of section 6; steps 3 to 6 become kkt-pro's work on its own tables and crates, with the core unchanged in key mode; the swarm input to `decide` (section 4 of the roadmap, and 12a's row for steps 4a and 4b) is dropped in favor of swarm-bound lookups; section 12a's crate table is redrawn by repository; and the wording in 8.3 and in `agent-credentials-design.md` 4.3 about atomic approval changes if the file-per-swarm design is chosen. `project-split-study.md` section 4.1, on the 32-kind mask, described the plan as written and no longer applies under this shape.
