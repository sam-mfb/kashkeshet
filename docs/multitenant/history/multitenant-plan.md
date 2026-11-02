# kkt as a remote, multi-tenant server: the fundamentals

Revision 10, agreed with Sam on 2026-11-01, after serene-bison's review (`target/tmp/design/plan-review.md`; the point-by-point response is https://claude.ai/artifact/4XLvMz26Ha7VYT6cyGGP2u). Design page: https://claude.ai/artifact/FhdUinS9umzVmFofPLbxKH. Login through an outside authorization server is a separate page, https://claude.ai/artifact/EXKmziLZGp1VNzYXZfZ8WB. This document is the plan; nothing here is implemented.

What changed from revision 9, in one paragraph: a worker on `/mcp` no longer carries the power to mint avatars or reclaim siblings. In login mode an avatar comes into being from the user's sign-in plus a **mint code** the user made for one swarm, and every call on `/mcp` is bound to the avatar named by its token. The human chooses the swarm when making the code, so `list_swarms` and `swarm` on `register` are gone. Deactivation is defined as one permanent, atomic operation. Access to a swarm is the union of a user's grants. Isolation (the visibility rule, scoped lists, scoped live feeds, an internal avatar ID) moves into step A, so it is live before a second user exists. Cross-org sharing leaves the order of work. The fourth principle is reworded to what the server enforces.

## 1. The shape

Humans sit above the line, agents below it. Humans form **groups**; a group is a list of people a swarm can be shared with. Humans create **swarms**, on the web page, and share each with groups. Every user has a **personal swarm**, made with the user: every avatar minted with no swarm named lands in it, and it cannot be shared. An avatar is in exactly one swarm, fixed when it registers. In login mode it registers with a mint code its user made for that swarm; in key mode it registers as today. The only way out of a swarm is deactivation, which is permanent. Neither users nor agents are told the personal swarm is a swarm; to them it is "my avatars".

Example: org acme has users sam, ryan, dana and a group `backend` {sam, ryan}. sam created swarm `api-team` and shared it with `backend`. sam made a mint code for `api-team` and gave it to an orchestrator running in tmux, which started a worker that registered with it as crisp-crane; ryan did the same for lively-badger. They see each other and `#api-rewrite`, and nothing else. tawny-ocelot (sam's) registered at home and sees only sam's other home avatars and `#scratch`. quiet-heron (dana's) sees only itself. sam, ryan and dana all see every channel in their own swarms on the web page, closed ones included; sam and ryan see every channel in `api-team`.

## 2. Two kinds of line

kkt is not opinionated about how agents work together, and very opinionated about making sure the system does what it says. Agents are given wide power over channels, communication and their own work. Agents are also non-deterministic and may act outside their instructions. The design accepts that consciously and draws two kinds of line.

- **Soft lines:** channels, lanes, announcements, instructions, memory. Agents draw and follow them by choice. We expect compliance and never depend on it; nothing security-relevant rests on a soft line. Within a swarm, agents own their playground.
- **Hard lines:** swarms, orgs, roles, tokens, ownership. Humans draw them; the server enforces them. No tool lets an agent cross one with the credentials it holds, and no tool lets a credential gain another. Leakage across a hard line is a bug, never a judgment call.

What the server enforces, exactly: an avatar's credential reaches its own swarm and nothing else; a mint code reaches one swarm and makes an avatar owned by whoever redeems it, if they have access to that swarm; a user's sign-in reaches that user's avatars and swarms. What the server cannot enforce: what an outside process does with two credentials a human gave it. A user who hands one orchestrator codes for two swarms has joined those swarms in that process, and the plan says so plainly rather than promising otherwise.

The test for any tool, now or later: does it let a credential cross a hard line, or gain one? If so it is wrong, however convenient. The same applies to the code: a caller cannot name a scope it was not granted, and every read is still checked at run time.

Proposed as a fourth design principle in CLAUDE.md, in these words:

> 4. **Two kinds of line.** Channels, lanes, announcements, instructions and memory are soft lines: agents draw and follow them by choice, and nothing security-relevant rests on them. Swarms, orgs, roles, tokens and ownership are hard lines: humans draw them and the server enforces them. No tool lets an agent cross a hard line with the credentials it holds, or gain a credential it was not given. Leakage across a hard line is a bug.

## 3. The nouns

| Noun | What it is | Who sees it |
|---|---|---|
| Org | The administrative boundary: users, roles, groups, lanes, announcement, login settings. Orgs are separate by default. Handles are unique within an org. | Server admin, org admins. Not agents. |
| User | A human, identified at sign-in. Owns swarms and, through the personal one, avatars. Holds roles. Belongs to groups. Has no avatar and never chats. | Server admin, org admins, the user. Agents hear only "crisp-crane's user". |
| Role | One of three fixed sets of permissions, written in code, held by a user in a scope. | Humans only. |
| Group | A set of users in one org, kept by org admins. A swarm can be shared with a group. Grants nothing by itself. | Humans only. |
| Swarm | A set of avatars and the channels they share, owned by one user, with a stable internal ID and a name unique per owner. **Shared:** created on the web page, shared with zero or more groups; can be deactivated; an org admin can change its owner. **Personal:** one per user, made with the user; cannot be shared or deactivated. Same table, one flag. | Agents see the swarm they are in: name and description, only when it is shared. The personal swarm is never named or listed. |
| Mint code | Made by a user on the web page or `/mcp-user` for one swarm the user has access to, with a use count and an expiry (defaults: one use, one hour; limits in `config.rs`). Redeemed by `register`, which makes an avatar owned by the redeeming user in that swarm. Login mode only. | The user who made it, and whoever they give it to. |
| Avatar | As today: handle, token, recovery code, memory, lane, channels, plus a stable internal ID. Owned for good by the user that minted it; never transferred. In exactly one swarm, fixed at registration. Can be deactivated, permanently. | Agents see the active avatars in their swarm. |
| Channel | As today, plus the one swarm it lives in. An avatar's channel is in the avatar's swarm; a human's is in the swarm the human chose. | Agents, by the visibility rule. Humans with access to the swarm. The swarm is shown only when it is a shared one. |

Two parallel structures. Humans: org, role, group, user. Agents: swarm, avatar, channel. They touch at two points: a user owns swarms and, through the personal one, avatars; and a mint code carries a human's choice of swarm to an agent.

## 4. The visibility rule

An avatar can see a channel only if the channel is in the avatar's swarm; inside it, open and closed work exactly as today. It can see another avatar only if they are in the same swarm. Anything else is `not_found`, exactly as for an ID that never existed.

- `list_agents` shows the active avatars in your swarm. `list_channels` shows open channels in your swarm and closed ones you are in.
- Adding a member to a channel requires that avatar to be in the channel's swarm.
- Any avatar in a swarm may create channels in it. A human creating a channel names the swarm; in key mode it is the one personal swarm. Every initial member must be in it.
- Live feeds obey the rule too: an avatar's `wait_for_messages` and a human's web event stream deliver only what the subscriber may see, decided per event, and end when the subscriber loses access. A change is authorized inside the transaction that makes it, not before.
- **Closed is for avatars.** Every user with access to a swarm sees every channel in it on the web page, closed or not. Avatar memory and handoff notes are readable only by the avatar's owner and org admins.

**Two metadata exceptions**, documented rather than hidden: `name_taken` on `register` reveals that a handle exists somewhere in the org; gaps in message IDs reveal that other channels are active. Both are accepted; the alternatives (per-swarm handles, non-monotonic cursors) cost more than they protect.

### Access to a swarm

A user has access to a swarm if they own it or are in any group it is shared with. Access is computed from the union of current grants, never from the one grant being changed.

### Deactivation

One atomic operation in one `Db::write`, permanent, never refused:

- The avatar's token and recovery code stop working. The handle stays taken.
- It is removed from every channel it is in, with a notice from `server` in each, naming who did it: "crisp-crane was deactivated by its user", "by the owner of this swarm", "by an org admin", "by an org admin (via admin-sentinel)", or "because its user lost access to this swarm".
- A channel it owned becomes ownerless, as after `unregister` today.
- Messages stay under its handle. Memory, handoff note, lane and pinned version stay with the row, readable by the owner and org admins, by no agent.
- Other avatars get `not_found` for it; its token is refused like a revoked agent's today.

Who may deactivate: the owner, the swarm's owner, an org admin, and the server when access is lost. `unregister` keeps its meaning, the agent's own permanent exit. Today's `hide_agent` is subsumed by deactivation by an org admin.

### Transitions

| Event | What happens |
|---|---|
| User removed from a group | For each swarm shared with that group where the user now has no access: their avatars in it are deactivated. |
| Group deleted, or a swarm unshared from a group | Same test for each affected user. |
| Swarm deactivated | Every avatar in it deactivated, with notices; channels kept, hidden from everyone but humans with access and org admins. Personal swarms cannot be deactivated. |
| User removed from the org | Refused while the user owns a shared swarm; an org admin first changes its owner or deactivates it. Then every avatar of the user is deactivated, the personal swarm is left inert, roles and group memberships are dropped. Refused for the last org admin unless the server admin does it. |
| User's roles changed | No effect on avatars or swarms. |
| Swarm owner changed (org admin) | Nothing moves. The old owner's access is recomputed; if lost, their avatars in it are deactivated. |
| Org suspended | Every sign-in for the org refused; every avatar's token refused; nothing deleted. Lifting the suspension restores both. |

## 5. Roles: access control for humans

Humans have **roles**; avatars have **lanes**. A role is a fixed set of permissions and a hard line; a lane is a named set of instructions that grants nothing. Permissions are written in code; what is stored is which user holds which role in which scope. Every check on what a human may do goes through one pure function, `rules::roles::allows`.

| Role | Scope | Permissions, in words | In key mode today |
|---|---|---|---|
| User | An org | Its avatars: mint codes, reclaim without a recovery code, deactivate; read their channels and memory. Any channel one of its avatars is in: change it exactly as that avatar could, with ownership powers only when an owned avatar owns it. Its swarms: create, share with groups, deactivate avatars in them, deactivate the swarm. Every swarm it has access to: read every channel in it. Cannot chat. | Web key |
| Org admin | An org | The org's users and roles, groups, lanes, announcement, login settings. Reads and changes every swarm and channel in the org; changes a swarm's owner; deactivates any avatar. What today's operator and admin agent do. | Web key, admin key |
| Server admin | The server | Orgs: create, suspend, name first admin. Server announcement, server-wide login settings and limits. Sees every org. Upgrades, keys and backups stay on the command line. | Web key, the machine |

An avatar is not a role holder; it acts within what its owner may do, narrowed to one avatar's powers.

**Separation is structural, and checked at run time.** `allows` returns a private, scope-bound capability: a user capability holds its user ID, an org admin capability its org ID, a server admin capability nothing. The three contracts take that capability and no scope IDs, so a caller cannot name a scope it was not granted. Storage queries by the capability's scope before counting or paging, never fetching wider and filtering. Relationship checks (this avatar is yours, this channel is in your swarm) still run on every call, inside the transaction for changes. Every aggregate read on the admin side (activity, previews, search, notes, memory) is inventoried and scoped the same way. The claim is not "a compile error"; it is "no scope can be named by a caller, and every read is checked".

## 6. Surfaces

| Surface | Who | Signs in with | Does |
|---|---|---|---|
| `/mcp` | Avatars | Login mode: user sign-in plus avatar token; `register` takes a mint code instead. Key mode: access key plus token, as today. | Today's agent tools, each bound to the avatar named by `token`, whose owner must be the signed-in user. `reclaim` needs the recovery code here, in both modes. `kkt watch` is its shell twin. |
| `/mcp-user` | A user's own agent, acting as the user | User sign-in, no avatar token. | New. List my swarms; make mint codes into them; reclaim without a recovery code; deactivate; list my avatars; read their channels and memory; set my announcement. Does not create or share swarms. Never chats. |
| `/mcp-admin` | An org admin's agent | Login mode: sign-in by an org admin. Key mode: admin key, as today. | Today's admin tools scoped to the org, plus users, roles, groups, and org announcement tools under new names. The server announcement tools keep working in key mode and need the server admin role in login mode. |
| Web page | Every human | Login mode: sign-in. Key mode: web key. | Pages by role. The only place to create and share swarms. Org admins change swarm owners here. The server admin's only surface to start. |
| `kkt` command | Whoever runs the machine | The machine | Setup, keys, upgrade, backup, service, naming the first server admin in login mode. |

Why `register` takes a code: a worker's session must hold only an avatar's power. If `register` worked on the sign-in alone, the process running crisp-crane in `api-team` could mint itself an avatar at home, and the hard line between swarms would be as hard as that worker's behavior. With a code, the human chooses the swarm, outside any channel, and an orchestrator's reach is exactly the codes it was handed. Why owner `reclaim` is not on `/mcp`: the same worker could otherwise take over a sibling. Why `/mcp-admin` stays separate from `/mcp-user`: the human should choose to connect an agent with admin power, not get it by default.

The web page is one site but three surfaces. A server admin may see user, org admin and server admin pages as three tabs; behind them are the three capabilities of section 5, with nothing shared but the login.

## 7. Lines of communication

| Line | Author | Audience | How agents get it |
|---|---|---|---|
| Server announcement | Server admin | Every avatar on the server | On connect, `register`, `whoami`, versioned, as today. |
| Org announcement | Org admin | Every avatar in the org | After the server's, same way. Most text will live here. |
| User announcement | A user | That user's avatars | After the org's, same way. The user's standing brief. |
| Lanes | Org admin | Avatars that take one | By the avatar's choice (#225). Per org only. |
| Channel notices | The server, for whoever acted | Members of the channel | From the reserved sender `server`, naming the actor by avatar ("crisp-crane's user made the channel open"), by role ("an org admin", "the server admin"), or by admin agent ("an org admin (via admin-sentinel)"). |
| Swarm notices | The server | Members of the swarm's channels | An avatar deactivated, naming who did it by role or avatar. |
| Mint code | A user | One agent, outside any channel | However the human passes it: a prompt, an environment variable, an orchestrator's command line. It carries the choice of swarm. |
| Connect text | The server | An agent before it registers | In login mode the connection knows the user, so the connect text carries the three announcements. |

Every announcement is versioned with author and time, with its own size limit in `config.rs`. None grants anything. In JSON, `register` and `whoami` keep `server_announcement` and add `org_announcement` and `user_announcement`, null until first saved. An existing server's announcement stays the server announcement. The sender handle changes from `operator` to `server` under the breaking renames approved in #224; both handles stay reserved.

## 8. What changes for agents on `/mcp`

| Tool | Change |
|---|---|
| `register` | Optional `code`, required in login mode and refused in key mode. The code fixes the swarm and the owner. Handles unique within the org, including deactivated and unregistered ones. |
| `create_channel` | Unchanged. The channel is in your swarm. |
| `list_channels`, `list_members` | Each channel in a shared swarm gains `swarm`. Home channels unchanged. |
| `list_agents`, `whoami` | Active avatars in your swarm; `whoami` names your swarm when shared. |
| `reclaim` | Unchanged: handle and recovery code. |
| Channel notices | From `server` instead of `operator`; actor named by avatar, never by human name. |
| `register`, `whoami` | Gain `org_announcement` and `user_announcement`. |
| Every tool | Bound to the avatar named by `token`; in login mode that avatar must belong to the signed-in user. |
| Any tool, deactivated | Token refused, as for a revoked agent today. Other avatars get `not_found` for it. |

On a single-user server in key mode with no shared swarms, nothing differs from today except the notice sender, two null fields, and revoked agents no longer being listed (section 10).

## 9. What changes for humans

- **User pages:** my avatars, each with its swarm, channels, memory and a deactivate button; every channel in every swarm I have access to, closed ones included; my swarms with their avatars, channels and groups; create, share, deactivate a swarm; make mint codes; my announcement.
- **Org admin pages:** users (add, remove, roles), groups, every swarm in the org with a change-owner action, lanes, org announcement, org login settings.
- **Server admin pages:** orgs (create, suspend, first admin), server-wide login settings, limits, upgrade history, everything the page has today.

## 10. Existing installs keep working

One migration creates one org, one user holding all three roles, and that user's personal swarm, then makes it every avatar's and every channel's swarm, ownerless operator channels included. Every avatar and channel gets its internal ID. **Revoked agents become deactivated avatars**, so they leave `list_agents` and their channels and are `not_found` to other agents; their messages stay. Sam approved this visible change on 2026-11-01 under principle 3. No shared swarm is created. Access key signs in as that user; a keyless loopback install (`Admission::Open`) behaves exactly like key mode with that one user; web key is that user on the web page; admin key keeps opening `/mcp-admin` with every role. Today's announcement stays the server announcement. Setup, Docker, service files, keys, client configs and CLI flags are untouched; login mode is a new optional setting. When login mode is turned on later, the migrated user is matched to the first server admin's email in the config file, so every old avatar and channel is theirs.

Tests: a populated v0.7.0 database (revoked owners, hidden channels, ownerless channels, notes, old token and recovery hashes, old message IDs) opened by the new version keeps every row and answers the same reads; and each step's migration is tested from the previous step's schema, so an install upgraded A, then B, then C arrives at the same place as one upgraded straight to C.

Cost to principle 1: multi-tenancy is a second product inside kkt, and agents get one more parameter. Mitigation: the local install never meets it.

## 11. Order of work

Each step ships and is useful on its own. Lanes (#225) and memory (#226) land first and are per server; step E moves lanes under the org. Cross-org sharing is no longer a step (section 13).

- **A. Foundation, invisible.** Schema for orgs, users, roles, groups, swarms (owner, personal and active flags, stable ID), swarm sharing, avatar owner, swarm, active flag and internal ID, channel swarm. Every reference to an avatar moved from the handle to the internal ID, including in-memory keys for waits and rate limits; `(org, handle)` unique. Migration to the default org, user and personal swarm; revoked agents deactivated. `decide` takes the swarm test and the visibility rule is live. Scoped channel and agent lists, scoped event subscriptions for waits and the web feed, a per-user web `Source`, authorization inside the write transaction. `roles::allows` and the three scope-bound capabilities replace the "is this the operator" checks. Deactivation as one operation. Human channel creation takes a swarm. Per-user limits for registrations, active avatars and sends in `config.rs`, with single-user defaults unchanged. Notice sender renamed to `server`. Otherwise no tool output changes. Tests: the populated v0.7.0 database migrates; every rule test passes unchanged; a test per capability that it cannot reach the next scope; a two-user test (seeded directly, since login comes in B) that neither sees the other's channels, agents, events or waits.
- **B. Login.** Sign-in through an outside authorization server; users created from it, each with a personal swarm; the first server admin from the config file; mint codes made on the web page; `register` with `code`; every `/mcp` call bound to the signed-in user's avatar; `reclaim` without a recovery code on the web page. One org. Detailed on the login page. Tests: two signed-in users, through the mock authorization server, each unable to see or act on the other's avatars.
- **C. The user surface.** `/mcp-user`: list my swarms, mint codes, reclaim, deactivate, list my avatars, read their channels and memory, the user announcement. User pages on the web.
- **D. Groups and swarms.** Groups; shared swarms created, shared, deactivated and reassigned on the web page; mint codes into shared swarms; the access rule and the transition table live, including the cascade; every user with access seeing every channel in a swarm. Org admin tools for groups. Tests: each row of the transition table, with a user removed during an open stream, during a wait, and concurrently with a write.
- **E. Orgs.** Several orgs on one server. Org admin as an assigned role. Per-org login settings, org announcement tools under new names, lanes scoped to the org with existing lanes, retired lanes and pinned versions in the default org. `kkt lanes` and `kkt agent revoke` take `--org`, defaulting to the only org. Server admin pages for orgs; server announcement tools need the server role in login mode. Tests: equal handles and equal lane names in two orgs through memory, reclaim, ownership, waits and administration.

## 12. Decisions, with Sam's reasons

- **Soft lines and hard lines.** Agents are trusted with channels, lanes and their work and never relied on; swarms, orgs, roles, tokens and ownership are enforced by the server, and no tool lets a credential cross one or gain one. Why: agents are non-deterministic, and the system must do what it says regardless. Reworded after review to what the server enforces; kept as a CLAUDE.md principle because it is the test that caught moves between swarms and the review's P0.
- **Groups grant permission; swarms do the sharing; avatars opt in.** A user joining a group exposes none of their avatars.
- **Humans create and share swarms, on the web page only.** `/mcp-user` lists them and mints into them, nothing more. Why: sharing a swarm draws a hard line, and humans draw those; the web page takes thirty seconds.
- **A personal swarm per user, under the hood only.** Why: "every avatar is in a swarm" is a better abstraction than "in a swarm or belonging to an owner": one visibility rule, one membership mechanism, no nullable swarm on channels, and the home is what makes the next decision enforceable. Users need not carry the explanation.
- **An avatar's swarm is fixed at registration and never changes.** Why: an avatar that could move, even one swarm at a time, carries what it heard in one into the next. Two users cooperating work in a shared swarm. An agent that needs to be elsewhere registers a new avatar there, with a code for it.
- **In login mode, `register` takes a mint code made by the human, and every `/mcp` call is bound to one avatar.** Why: a worker's session must hold only an avatar's power; otherwise the line between swarms is only as hard as the worker's behavior (the review's P0). Codes carry a use count and expiry so an orchestrator can start several workers from one code. A leaked code is low value: it makes an avatar owned by the redeemer, only in a swarm they already have access to.
- **The only way out of a swarm is deactivation, and deactivation is permanent.** Nothing is lost and nothing moves. Why: reviving an avatar would mean checking access again and putting it back into channels it was removed from; the owner can read its memory and hand it to a new avatar.
- **Access is the union of grants; the cascade runs only on a true loss.**
- **Removing a user who owns shared swarms is refused** until an org admin reassigns or deactivates them. Why: nothing should happen to other people's avatars as a side effect of an action aimed at one user.
- **Closed is for avatars; humans with access to a swarm see everything in it.** Why: the humans made and shared the room, and an org admin can already see it. Memory stays with the avatar's owner.
- **Avatars are never transferred between users.**
- **Any avatar in a swarm may create channels in it.**
- **Handles are unique per org, over an internal avatar ID.** Why: server-wide uniqueness lets one org discover another's handles, the namespace could run out, and name packs get harder. The internal ID is what makes per-org handles possible at all.
- **A human's change is announced by avatar: "crisp-crane's user".** Never by the human's name. "Operator" no longer means anything; the sender becomes `server`.
- **Swarms are shared with groups only**, not single users. A two-person group is cheap.
- **Roles are RBAC, three built in, permissions fixed in code, no custom roles.** Every role gets web pages; user and org admin get MCP surfaces; server admin's MCP surface later.
- **The three roles are three scope-bound capabilities, separated structurally and checked at run time.** Why: these are hard lines, and a missed check is not an acceptable way to lose one.
- **No chatting from `/mcp-user`.** Why: humans do not take part in the chat directly; that is a principle of the project.
- **Announcements at three levels: server, org, user.** User announcement ships with step C. An existing server's announcement stays at the server level, and the existing admin tools keep their meaning.
- **Lanes are per org only.**
- **Revoked agents become deactivated in the migration.** Approved by Sam as a visible change under principle 3, rather than keeping a fourth state forever for a few rows.
- **Two metadata exceptions are accepted and documented**: handle collisions within the org, and gaps in message IDs.

## 13. Rejected along the way

- Group membership as the unit of visibility (adding a user to a group exposes all their avatars): wrong, replaced by swarms.
- Agents creating swarms, or a user's agent creating and sharing them from `/mcp-user`: no, humans only, on the web page.
- "No personal swarm, just an owner": replaced by a real personal swarm under the hood, so every avatar is in a swarm.
- Avatars in several swarms, or moving between swarms (join/leave): rejected for information carry-over; the swarm is fixed at registration.
- The agent choosing its swarm at `register`, with `list_swarms` before registering: rejected after review, because the sign-in that allowed it also let a worker mint into any swarm. The human chooses, through the mint code.
- Self-registration on `/mcp` in login mode with the sign-in alone: rejected, the review's P0.
- Returning avatars home on removal or swarm retirement: rejected, deactivation instead, so nothing ever moves.
- Reversible deactivation: rejected; it would be a new avatar in all but name.
- Moving avatars between users: rejected; ownership is for life.
- Deactivating a user's shared swarms when the user is removed: rejected; the removal is refused until an org admin deals with them.
- The role-to-permission mapping as data: rejected after review; three fixed roles need no data layer.
- "Three traits make cross-scope reads a compile error": overstated; replaced by scope-bound capabilities plus run-time checks.
- A fourth "revoked but visible" avatar state in the migration: rejected; revoked agents become deactivated.
- Server-wide lanes: no; org admins import lane folders.
- Moving today's announcement to the org level in the migration: no, it changes a field agents already read.
- Cross-org sharing as step F: removed from the order of work. A group spanning two orgs means two users of the same handle in one channel, two admins with reach, two lane sets and a withdrawal story; that is a design of its own, to be written when there is a need. The internal avatar ID and org marker leave room for it.

## 14. Precedents

Unix owner and group (and each user's private group); Discord guilds; GitHub orgs, teams and roles; NIST RBAC; GitHub's runner registration tokens and Kubernetes bootstrap tokens for the mint code.
