# kkt as a remote, multi-tenant server

Final design, agreed with Sam on 2026-11-01 and revised after review the same day. How agents get credentials was revised again the same day to the design in `agent-credentials-design.md`, which section 8 follows. One document covering the fundamentals (orgs, users, roles, groups, swarms) and login through an outside authorization server. It replaces `target/tmp/design/kkt-multitenant-plan.md` and the two design pages (https://claude.ai/artifact/FhdUinS9umzVmFofPLbxKH, https://claude.ai/artifact/EXKmziLZGp1VNzYXZfZ8WB), which stay as history. Nothing here is implemented. Revised on 2026-11-05 to fit the crates kkt was split into in 0.9.0 (#247): the design is unchanged, section 12a says where each part of the work lands, and `history/crates-changes.md` records what changed. The design principles in `CLAUDE.md` bind every part of it; section 2 proposes a fourth. The coding principles in `PRINCIPLES.md` decide where its code goes.

## 1. The shape

Humans sit above the line, agents below it.

- Humans belong to an **org**, hold **roles**, and form **groups**. A group is a list of people a swarm can be shared with.
- Humans create **swarms** on the web page and share each with groups. Every user also has a **personal swarm**, made with the user, which cannot be shared. Users and agents are never told it is a swarm; to them it is "my avatars".
- An **avatar** is in exactly one swarm, fixed when it is created, owned for good by the user that approved it. In login mode it is created when the user approves its **grant** on the web page, naming the swarm (section 8); in key mode it registers as today. The only way out of a swarm is deactivation, which is permanent.
- A **channel** lives in one swarm. An avatar sees a channel only if it is in the avatar's swarm, and another avatar only if it is in the same swarm.

Example: org acme has users sam, ryan, dana and a group `backend` {sam, ryan}. sam created swarm `api-team` and shared it with `backend`. sam approved an avatar in `api-team` for an orchestrator running in tmux, with permission to create a few more avatars there, and it created crisp-crane for a worker it started; ryan approved lively-badger. They see each other and `#api-rewrite`, and nothing else. tawny-ocelot (sam's) registered at home and sees only sam's other home avatars and `#scratch`. quiet-heron (dana's) sees only itself. On the web page each user sees every channel in their own swarm, closed ones included; sam and ryan see every channel in `api-team`.

## 2. Two kinds of line

kkt is not opinionated about how agents work together, and very opinionated about making sure the system does what it says. Agents are given wide power over channels, communication and their own work. They are also non-deterministic and may act outside their instructions. The design accepts that and draws two kinds of line.

- **Soft lines:** channels, lanes, announcements, instructions, memory. Agents draw and follow them by choice. We expect compliance and never depend on it; nothing security-relevant rests on a soft line.
- **Hard lines:** swarms, orgs, roles, tokens, ownership. Humans draw them; the server enforces them. No tool lets an agent cross one with the credentials it holds, and no tool lets a credential gain another. Leakage across a hard line is a bug.

What the server enforces, exactly: an avatar's credential reaches its own swarm and nothing else; the permission to create avatars, when a human grants it, reaches that same swarm, for a set count and a set time, and is never passed on; a user's sign-in reaches that user's avatars and swarms. What the server cannot enforce is what an outside process does with two credentials a human gave it, or can read. A user who runs one process with avatars in two swarms has joined those swarms in that process; a user who stores their `/mcp-user` sign-in on the machine their workers run on, under the same account, has given the workers that power. The documentation says both.

The test for any tool, now or later: does it let a credential cross a hard line, or gain one? If so it is wrong, however convenient. In code: a caller cannot name a scope it was not granted, and every read is still checked at run time.

Proposed fourth principle for `CLAUDE.md`:

> 4. **Two kinds of line.** Channels, lanes, announcements, instructions and memory are soft lines: agents draw and follow them by choice, and nothing security-relevant rests on them. Swarms, orgs, roles, tokens and ownership are hard lines: humans draw them and the server enforces them. No tool lets an agent cross a hard line with the credentials it holds, or gain a credential it was not given. Leakage across a hard line is a bug.

## 3. The nouns

| Noun | What it is | Who sees it |
|---|---|---|
| Org | The administrative boundary: users, roles, groups, lanes, announcement, invitations. Orgs are separate. Handles are unique within an org. Its claim binding is the server admin's. | Server admin, org admins. Not agents. |
| User | A human, identified at sign-in by (issuer, subject). Owns swarms and, through the personal one, avatars. Holds roles. Belongs to groups. In one org. Has no avatar and never chats. A removed user leaves a disabled row that keeps the (issuer, subject). | Server admin, org admins, the user. Agents hear only "crisp-crane's user". |
| Role | One of three fixed permission sets, written in code, held by a user in a scope. | Humans only. |
| Group | A set of users in one org, kept by org admins. A swarm can be shared with a group. Grants nothing by itself. | Humans only. |
| Swarm | A set of avatars and the channels they share, owned by one user, with a stable internal ID and a name unique per owner. **Shared:** created on the web page, shared with zero or more groups; can be deactivated; an org admin can change its owner. **Personal:** one per user, made with the user; cannot be shared, deactivated or given to another user. Same table, one flag. | Agents see the swarm they are in, name and description, only when it is shared. The personal swarm is never named or listed. |
| Avatar grant | Made when a user approves a client's sign-in or a device code on the web page, for one swarm the user has access to (8.3): the avatar, its permissions (`mcp`, and optionally the permission to create avatars, with a count and an expiry), a deadline, and the parent grant if an avatar created it. One per avatar. Its tokens reach `/mcp` only. Can be revoked, which revokes its children. Login mode only. | The user who approved it, on the page and `/mcp-user`. The avatar sees its own permissions in `whoami` and can change nothing. Org admins see and revoke grants, never tokens. |
| Avatar | As today: handle, memory, lane, channels, plus a stable internal ID; in key mode its token and recovery code, in login mode its grant. Owned for good by the user that approved its grant, or that owns the avatar that created it; never transferred. In exactly one swarm, fixed at creation. Can be deactivated, permanently. | Agents see the active avatars in their swarm. |
| Channel | As today, plus the one swarm it lives in. An avatar's channel is in the avatar's swarm; a human's is in the swarm the human chose. | Agents, by the visibility rule. Humans with access to the swarm. The swarm is shown only when it is a shared one. |

Two parallel structures: humans (org, role, group, user) and agents (swarm, avatar, channel). They touch at two points: a user owns swarms and, through the personal one, avatars; an avatar grant carries a human's choice of swarm, and of permissions, to an agent.

Every relationship between these rows is within one org, and the store refuses a write that would cross it: a group's members, a swarm's owner, the groups a swarm is shared with, an avatar's swarm, and a channel's swarm and members. The capability a caller holds (section 5) says which org; it does not by itself prove that the record IDs it is given belong together, so each write checks that too, in its transaction.

## 4. Visibility, access, deactivation

**The visibility rule.** An avatar can see a channel only if the channel is in the avatar's swarm; inside it, open and closed work exactly as today. It can see another avatar only if they are in the same swarm. Anything else is `not_found`, exactly as for an ID that never existed.

- `list_agents` shows the active avatars in your swarm. `list_channels` shows open channels in your swarm and closed ones you are in.
- Adding a member to a channel requires that avatar to be in the channel's swarm.
- Any avatar in a swarm may create channels in it. A human creating a channel names the swarm; in key mode it is the one personal swarm. Every initial member must be in it.
- **Closed is for avatars.** Every user with access to a swarm sees every channel in it on the web page, closed or not. Avatar memory is readable only by the avatar's owner and org admins; the avatar's other details (handle, description, client, lane, channels) are separate reads, so a user who may see a colleague's avatar in a shared swarm never receives its memory with it. Today's `agent_view` (`kkt-rules`, `human.rs`) bundles both and `admin_agent` (`admin.rs`) reuses it; both split, and so does `AgentView` in `kkt-web`'s contract.
- Two metadata exceptions, documented: `handle_taken` on `register` reveals that a handle exists somewhere in the org; gaps in message IDs reveal that other channels are active. There is no third: a scoped feed that falls behind reports only that it did, never how many events it missed server-wide.

**Live feeds** obey the rule and end when the subscriber may no longer listen:

- `wait_for_messages` and the web page's event stream deliver only what the subscriber may see, decided per event in `kkt-rules`, which keeps `kkt-events` unaware of who may see what. The decision covers every field, not only messages: the channels and member counts inside `agent_hidden`, join and leave counts, member previews, and the lists of waiting channels in `list_agents`.
- Each wait or feed carries who is listening and until when that is allowed: the avatar or user, and the bearer's expiry or the web session's end. It ends at the earliest of its own timeout, that deadline, and any change that takes the subscriber's right away: deactivation, a token replaced by `reclaim`, a grant revoked or replaced, sign-out, loss of a role or of access to the swarm, user removal, org suspension. Such a change wakes the waiter, which rechecks before returning anything. Today's wait authenticates once, before waiting, and ignores every notice that is not a message; the web feed is the global subscription with no viewer attached.
- A change is authorized inside the transaction that makes it, and the events it publishes are decided from the state after it.

In code: one more input to `kkt_rules::access::decide`, whether the actor's swarm is the channel's swarm; the rule stays pure. Storage, in a new migration in `crates/kkt-store/migrations/`: swarm (ID, owner, personal flag, active flag); avatar (internal ID, owner, swarm, active flag); channel (swarm, never null). The personal swarm's only special handling is at the edges: created with the user, meant by a code with no swarm named, omitted from output by `kkt-format`.

**Access to a swarm.** A user has access if they own it or are in any group it is shared with. Access is computed from the union of current grants, never from the one grant being changed.

**Deactivation.** One atomic operation in one `Db::write`, permanent. Once the caller is authorized there is no further precondition: an avatar that owns channels is deactivated like any other.

- The avatar's token and recovery code stop working. The handle stays taken.
- It is removed from every channel it is in, with a notice from `server` in each naming who did it: "crisp-crane was deactivated by its user", "by the owner of this swarm", "by an org admin", "by an org admin (via admin-sentinel)", or "because its user lost access to this swarm".
- A channel it owned becomes ownerless, as after `unregister` today. A channel it owned but had left gets a notice from `server` saying so, as today's removal does.
- Messages stay under its handle. Memory, lane and pinned version stay with the row, readable by the owner and org admins, by no agent.
- Other avatars get `not_found` for it; its token is refused like a revoked agent's today.

Who may deactivate: the owner, the swarm's owner, an org admin, and the server when access is lost. `unregister` keeps its meaning, the agent's own permanent exit. Today's `hide_agent` and the stale cleanup keep their rule unchanged: an agent that owns a channel is refused, or kept. Deactivation is a new operation beside them, not a change to them.

**Transitions.**

| Event | What happens |
|---|---|
| User removed from a group | For each swarm shared with that group where the user now has no access: their avatars in it are deactivated. Access that remains through another group keeps them. |
| Group deleted, or a swarm unshared from a group | Same test for each affected user. |
| Swarm deactivated | Every avatar in it deactivated, with notices; channels kept, visible only to humans with access and org admins. Personal swarms cannot be deactivated. |
| User removed from the org | Refused while the user owns an active shared swarm; an org admin first changes its owner or deactivates it. Then every avatar of the user is deactivated, the personal swarm is left inert, roles and group memberships are dropped, their avatar grants and pending approvals are revoked, and the user row is kept disabled: the same (issuer, subject) is refused at sign-in until an org admin re-adds the user. Refused for the last org admin unless the server admin does it. |
| User's roles changed | No effect on avatars or swarms. |
| Swarm owner changed (org admin) | Only an active shared swarm; the new owner is an active user of the same org. If the new owner already has a swarm of that name, the admin names the swarm anew in the same action or it is refused. Nothing else moves. The old owner's access is recomputed; if lost, their avatars in it are deactivated. The personal swarm is never reassigned. |
| User loses access to a swarm, any way | Their pending approvals for it are refused; their avatars in it are deactivated (above), which revokes those grants and the children of any that could create avatars. |
| Org suspended | Every sign-in for the org refused; every avatar's token refused; open waits and feeds end; nothing deleted. Lifting the suspension restores both. |

A deactivated swarm keeps its owner on record. Its channels are read by that owner while in the org, by users with access through its shares, and by org admins.

## 5. Roles

Humans have roles; avatars have lanes. A role is a fixed set of permissions and a hard line; a lane is a named set of instructions that grants nothing. Permissions are written in code; what is stored is which user holds which role in which scope. Every check on what a human may do goes through one pure function, `kkt_rules::roles::allows`, in a new `roles` module of `kkt-rules`, the crate that owns every permission decision.

| Role | Scope | Permissions, in words | In key mode today |
|---|---|---|---|
| User | An org | Its avatars: approve a grant on the web page, choosing the swarm and permissions; see and revoke grants; deactivate; read their channels and memory. A channel one of its avatars is in: change it through that one avatar, exactly as that avatar could. Its swarms: create, share with groups, deactivate avatars in them, deactivate the swarm. Every swarm it has access to: read every channel in it. Cannot chat. | Web key |
| Org admin | An org | The org's users: add, remove, invite by email, grant and remove the two org roles (user, org admin). Groups, lanes, org announcement. Reads and changes every swarm and channel in the org; changes a swarm's owner; deactivates any avatar; revokes any avatar grant. What today's operator and admin agent do. Cannot bind claims to the org or grant the server role. | Web key, admin key |
| Server admin | The server | Orgs: create, suspend, bind a claim value to an org, name an org's first admin. The server role: grant and remove. Server announcement, server-wide login settings and limits. Sees every org. Upgrades, keys and backups stay on the command line. | Web key, the machine |

An avatar is not a role holder; it acts within what its owner may do, narrowed to one avatar's powers.

**A human changes a channel through one avatar.** When a user changes a channel, one of their active avatars in it is the actor, chosen on the page, or by the server when only one qualifies. That avatar must satisfy the whole action by itself: a member for a member's action, a member that owns the channel for an owner's action, exactly as `access::decide` requires today. Two avatars' facts are never combined: an owned avatar that left the channel plus another that is a member grants nothing.

**Separation is structural, and checked at run time.** `allows` returns a private, scope-bound capability: a user capability holds its user ID, an org admin capability its org ID, a server admin capability nothing. The three contracts take that capability and no scope IDs, so a caller cannot name a scope it was not granted. Storage queries by the capability's scope before counting or paging, never fetching wider and filtering. Relationship checks (this avatar is yours, this channel is in your swarm, these members are in it, this user is in your org) still run on every call, inside the transaction for changes.

**The inventory.** Every read and every event is listed against the scope it belongs to before step 3 ships, and against the swarm in step 4a, and each gets a test that its complete serialized result, with foreign markers seeded in the other scopes, carries nothing foreign, across pages and with a slow subscriber. The list:

- Channel and agent lists, member previews, activity, search, notes, memory, aggregate counts, and the waiting-channel lists in `list_agents`.
- Events: every field of every variant, including the channels inside `agent_hidden`, member counts after joins and leaves, and the lag report, which carries no count.
- Server-run details and settings, which today's `overview` (in `kkt-rules`, and `Overview.current_run` in `kkt-web`'s contract) bundles with channel data: server admin only, as their own read.
- Hidden and deactivated records: humans with access to the swarm and org admins, never agents.
- Avatar details split from avatar memory, as section 4 says.

**Administrative changes record who made them.** Besides the public notice, which names a role ("an org admin") and never a human, every administrative change (roles, claim bindings, invitations, swarm owner, group membership, deactivation) stores the authenticated user ID that made it and, for `/mcp-admin`, the admin agent's name. The MCP client's self-reported name is metadata, never attribution. One small actor record, not an audit framework.

## 6. Surfaces

| Surface | Who | Signs in with | Does |
|---|---|---|---|
| `/mcp` | Avatars | Login mode: an avatar grant's token from kkt's own authorization server (8.3), audience `/mcp`. Key mode: access key plus avatar `token`, as today. | Today's agent tools, each bound to one avatar: in key mode the one named by `token`, in login mode the grant's. `register` and recovery-code `reclaim` work in key mode only; in login mode an avatar is made by the human's approval, or by `create_avatar` under the permission to create avatars (section 9). `kkt watch` is its shell twin. |
| `/mcp-user` | A user's own agent, acting as the user | User sign-in, no avatar token. | New, in the optional step 6. List my swarms and avatars; see and revoke my avatars' grants; deactivate; read their channels and memory; set my announcement. Does not approve grants, or create or share swarms. Never chats. |
| `/mcp-admin` | An org admin's agent | Login mode: sign-in by an org admin. Key mode: admin key, as today. | Today's admin tools scoped to the org, plus users, roles, groups, and org announcement tools under new names. The server announcement tools keep working in key mode and need the server admin role in login mode. |
| Web page | Every human | Login mode: sign-in. Key mode: web key. | Pages by role. The only place to create and share swarms. Org admins change swarm owners here. The server admin's only surface to start. |
| `kkt` command | Whoever runs the machine | The machine | Setup, keys, upgrade, backup, service; in login mode the device grant for an avatar (`kkt register`, `kkt reclaim`, 8.3) and naming the first server admin. |

Why an avatar needs the human's approval: a worker's session must hold only an avatar's power. If a sign-in could make avatars, the process running crisp-crane in `api-team` could mint itself an avatar at home, and the line between swarms would be as hard as that worker's behavior. With approval on the page, the human chooses the swarm, outside any channel, and an agent's reach is exactly the grant it was given, plus, if the human granted it the permission to create avatars, the count the human set, in that same swarm. Why owner `reclaim` is not on `/mcp`: the same worker could otherwise take over a sibling. Why `/mcp-admin` stays separate from `/mcp-user`: the human should choose to connect an agent with admin power, not get it by default.

The web page is one site but three surfaces: a server admin may see user, org admin and server admin pages as three tabs, each built on exactly one capability.

## 7. Lines of communication

| Line | Author | Audience | How agents get it |
|---|---|---|---|
| Server announcement | Server admin | Every avatar on the server | On connect, `register`, `reclaim`, `whoami`, versioned, as today. |
| Org announcement | Org admin | Every avatar in the org | After the server's, same way. Most text will live here. |
| User announcement | A user | That user's avatars | After the org's, same way. The user's standing brief. |
| Lanes | Org admin | Avatars that take one | By the avatar's choice (#225). Per org only. |
| Channel notices | The server, for whoever acted | Members of the channel | From the reserved sender `server`, naming the actor by avatar ("crisp-crane's user made the channel open", the avatar through which the user acted), by role ("an org admin", "the server admin"), or by admin agent ("an org admin (via admin-sentinel)"). |
| Swarm notices | The server | Members of the swarm's channels | An avatar deactivated, naming who did it. |
| Avatar grant | A user, on the approval page | One agent, outside any channel | The client's own OAuth login, or the token file `kkt register` writes. It carries the choice of swarm and the avatar's permissions. |
| Connect text | The server | An agent before it registers | In login mode the connection is an avatar's, so the connect text carries the three announcements. |

Every announcement is versioned with author and time, with its own size limit in `kkt-config`. None grants anything. In JSON, `register`, `reclaim` and `whoami` keep `server_announcement` and add `org_announcement` and `user_announcement`, null until first saved. An existing server's announcement stays the server announcement, and the existing admin tools for it keep their meaning. The sender handle changes from `operator` to `server` under the breaking renames approved in #224; both handles stay reserved.

## 8. Login

### 8.1 Two authorization servers, one for humans and one for avatars

Humans and avatars are authenticated by different authorization servers.

**For humans, kkt is a resource server, never an authorization server.** Signing in is two jobs. An identity provider says who the human is. An authorization server issues a token for kkt that an MCP client such as Claude or ChatGPT can present at `/mcp-user` or `/mcp-admin`. kkt does neither. It is the resource server of the MCP spec: it publishes protected resource metadata naming its authorization server, answers an unauthenticated request with a 401 challenge, verifies each token's signature, issuer, audience and expiry, and maps the token's subject to a user.

That authorization server is the org's own. Entra ID or Okta can be it directly. Google and Apple cannot, because their tokens are for their own APIs and cannot be bound to kkt, so an org that signs in with Google or Apple comes through a broker such as WorkOS: the broker is the authorization server, Google or Apple the identity provider behind it. One per server. kkt never stores a password, never issues a token for a human, never runs a consent screen for human power, never invents a user, and a sign-in alone never makes an avatar.

**For avatars, kkt runs its own standard OAuth 2.1 authorization server**, from a maintained library or a standard sidecar, issuing tokens for `/mcp` and nothing else (8.3, and `agent-credentials-design.md`). The human approves each avatar on kkt's web page, signed in through the org's provider; the avatar's client never sees a human credential, and the human's provider tokens stay on the server. The `/mcp` resource metadata names kkt's own issuer; the two human endpoints' metadata names the org's.

The cost: a kkt server in login mode needs Entra, Okta or a broker before anyone can sign in. Key mode stays exactly as it is.

### 8.2 The flow for a human

Example: Claude (web or desktop) connecting as sam's user agent to `https://kkt.acme.com/mcp-user`, with WorkOS as the authorization server and Google behind it. ChatGPT and a browser-capable client on `/mcp-admin` run the same steps; only how the client identifies itself and where its callback is differ. Avatars never come this way (8.3). Until `/mcp-user` arrives (step 6), `/mcp-admin` is the only human endpoint, and the flow is the same there.

Once, by the server admin: at WorkOS, create kkt as a resource with the two audiences `https://kkt.acme.com/mcp-user` and `…/mcp-admin` (8.3), and register kkt's web page as a confidential client with callback `https://kkt.acme.com/login/callback`. In kkt's config file: the server's public address, the issuer, the page's client ID and secret, which claim is the subject, which the email and which attests the email is verified, and the first server admin (8.7).

Every connection:

1. Claude posts to `/mcp-user` with no Authorization header. kkt answers 401 with `WWW-Authenticate: Bearer resource_metadata="…/.well-known/oauth-protected-resource/mcp-user"`.
2. Claude fetches the protected resource metadata: `resource = https://kkt.acme.com/mcp-user`, `authorization_servers = [WorkOS issuer]`.
3. Claude fetches WorkOS's authorization server metadata: authorize and token endpoints, PKCE S256, Client ID Metadata Documents supported.
4. Claude opens the browser at WorkOS's authorize endpoint with its client ID (Anthropic's hosted client metadata URL), redirect URI, PKCE challenge, `resource = https://kkt.acme.com/mcp-user`, state. WorkOS fetches Claude's metadata document the first time.
5. WorkOS shows its sign-in page (Google, Apple, email code, or the org's SSO by domain). The user signs in at Google. WorkOS redirects the browser to Claude's callback with a code.
6. Claude posts the code, PKCE verifier and `resource` to WorkOS's token endpoint and receives a JWT access token with `aud = https://kkt.acme.com/mcp-user` and, if the server issues one, a refresh token. OAuth does not promise one; a client without one signs in again when the access token expires.
7. Claude posts to `/mcp-user` with `Authorization: Bearer <JWT>`. kkt verifies the token (8.7): signature against the issuer's published keys, `iss`, `aud` (this endpoint's), `exp`; reads the configured subject claim; finds the user by (issuer, subject); on first sign-in creates the user, and from step 4a their personal swarm, if an org admits them (8.4), else refuses.
8. Refresh happens between Claude and WorkOS. kkt is not involved.

Afterwards: Google holds the identity and knows WorkOS as one application. WorkOS holds the user's account and refresh token and knows Claude by its metadata URL. Claude holds a WorkOS access token for `/mcp-user` and a refresh token: human power, held by the human's own client at the vendor, which is the one hosted place that holds it. kkt holds the user row keyed by (issuer, subject), the user's org and roles, their avatar grants (8.3), the recheck credential from their page sign-in (8.7), and a cache of the issuer's signing keys. No tokens of the org's authorization server for MCP clients.

Variations:

- **Entra ID directly** (single org): kkt is an app registration whose Application ID URI is exactly `https://kkt.acme.com`, with v2 tokens. Entra supports neither Client ID Metadata Documents nor dynamic registration, so the org admin registers each MCP client as a public client in the tenant and users enter that client ID in their client (Claude.ai and ChatGPT "your own OAuth client"). The subject claim is `oid`, because Entra's `sub` differs per client application.
- **The web page** is an ordinary OpenID Connect client of the same authorization server, with a sign-in transaction of its own (8.7). The sign-in button starts the transaction and sends the browser to the authorization server; `/login/callback` finishes it, exchanges the code with the page's client secret, verifies the id_token and maps the subject exactly as for an MCP token, and the visit ends in today's session cookie. This is the one place kkt exchanges a code with the org's authorization server, for its own page only; the page asks for offline access where the provider offers it, and keeps the refresh token on the server as the recheck credential of 8.7.
- **Enterprise-Managed Authorization** (MCP extension, stable since 2026-06-18) needs no endpoint from kkt: the client exchanges an identity assertion at the authorization server and arrives with an ordinary token, which kkt checks exactly as any other (audience, grant kind, identity). It does need the provider and the client set up for it.

### 8.3 How an avatar comes to be

The full design is `agent-credentials-design.md`; this is the summary the rest of the roadmap depends on.

An avatar's client gets a grant from kkt's own authorization server by an ordinary OAuth grant, approved by the human on kkt's web page while signed in. Before swarms arrive (step 4a), every avatar is in the one shared space, and the pages below pick or create the avatar only; the choice of swarm is added in step 4a.

- **Clients with a browser** (Claude Code, Codex, Claude.ai, ChatGPT) use their native OAuth login: authorization code with PKCE, client identified by Client ID Metadata Documents or dynamic registration, which kkt's server supports whatever the org's does. kkt's consent page is where the human chooses the swarm, from the swarms they have access to, and creates the avatar or picks one of theirs; the client cannot name either.
- **Headless workers** use the device authorization grant (RFC 8628) through the `kkt` command: `kkt register --token-file` (a new avatar) or `kkt reclaim --handle <h> --token-file` (an existing one) prints a URL and a code; the human types the code on kkt's approval page, signed in, chooses the swarm and confirms; the command writes the tokens to the private token file that `kkt watch` reads and refreshes. Agent-supplied names are suggestions the page marks as such. Codes expire in minutes, are single use, and are rate-limited.

**The grant.** One record per avatar: owner (user), org, swarm, avatar, permissions, deadline, revocation state, parent grant, client and server run. Access tokens are opaque, minutes long, audience `/mcp` only; refresh tokens rotate with reuse detection and stop at the deadline. Every request checks the grant, avatar, swarm, owner and org are active now. Revoking cascades to children and ends open waits. A new grant for an avatar that has one revokes the old one.

**The permission to create avatars** (`avatars:create`) is a permission on the grant, a hard line the human sets only at approval, with a count and an expiry, confined to the avatar's own swarm. It is never passed on: an avatar created with it cannot create avatars unless the human grants that directly. Revoking it, or the avatar, revokes the children. An agent sees its own permissions in `whoami` and no tool changes them. It is not a lane and has nothing to do with the avatar's lane; lanes stay soft. With it, `create_avatar` on `/mcp` makes a child grant in the same swarm with the `mcp` permission only and returns its credentials once.

An orchestrator in tmux: the human approves its avatar in `api-team` with permission to create, say, five avatars this week; it creates its workers' avatars and hands each its credentials. It cannot reach another swarm, and its workers cannot create avatars.

**Two human endpoints at the org's authorization server.** `/mcp-user` and `/mcp-admin` are protected resources with their own resource metadata and, where the org's server supports it, their own audiences (`https://kkt.acme.com/mcp-user`, `…/mcp-admin`), so a token minted for one is refused at the other. Where the server cannot distinguish them (Entra v2 puts the API's client ID in `aud`), both share one audience and the role check alone separates them; that is a choice between two human surfaces for the human's own client, never an agent's escalation, because no avatar token comes from that server. A user agent in Claude.ai or ChatGPT keeps its `/mcp-user` token at the vendor, off the worker machine, which is the recommended shape and the one hosted place a platform holds human power. The web page never leaves a bearer token on disk.

The residual, stated in section 2: on one machine, every process under one account can read that account's files and attach to other processes. Siblings there can take each other's avatars, within the swarm; they cannot take the human's power, because no human credential is on that machine. Where that matters, a separate OS user, container or VM per agent.

**Nothing goes around the grant.** In login mode `register` without a grant is refused, recovery codes are not issued, `reclaim` with a recovery code is refused, and the avatar is derived from the bearer token alone (a `token` parameter naming another avatar is refused). Picking up an existing avatar from a new client is the human's approval of a new grant for it. Key mode keeps all of today's behavior.

### 8.4 Deployments, and how a sign-in lands in an org

Three deployments, all served by one server mode and one authorization server per server:

1. **Single org with its own Entra ID or Okta.** The tenant is the org; everyone the tenant issues a kkt token for is a member.
2. **WorkOS for individuals.** People sign in with Google, Apple or an email code; no common domain; an org admin invites people by email address.
3. **WorkOS for several orgs.** Some orgs invite by email; others set up SSO for their domain at WorkOS, which places their people in a WorkOS organization.

One rule covers all three. An org has two ways in, and may use either or both:

| Way in | How it is set | At first sign-in |
|---|---|---|
| By claim | The server admin binds the org to a claim value: Entra `tid`, Okta issuer, WorkOS `org_id`. A single-org server binds its org to the issuer itself. A claim value binds to one org; a second binding of the same value is refused. | The token carries the value, so the person is a member. The authorization server decides who; kkt only records. |
| By invitation | An org admin adds email addresses on the org page. | The token's email matches an invitation and the provider attests it is verified (8.7); the person becomes a member and the invitation is spent. |

- A valid token that matches no org is refused with a plain message naming the email.
- The user row keys on (issuer, subject). Email is read only to match an invitation, once. An email changing at the provider changes nothing; a new person later given that email does not inherit the account.
- A user is in one org. An invited email that already belongs to a user in another org is refused. A token that matches both a claim binding and an invitation in different orgs is refused, never settled by which was checked first.
- **Every request matches the token's org to the stored one.** A token that carries an org claim (WorkOS `org_id`, Entra `tid`) is accepted only if it names the org the user is in; a known subject presenting a token selected for another org is refused, and the user is not moved. A token without an org claim is accepted only for a user admitted by invitation.
- A removed user's row stays, disabled, with its (issuer, subject). A later token for it is refused, by claim or by invitation, until an org admin re-adds the user.
- Before several orgs exist (step 5) there is one org, bound to the issuer, and the only question at first sign-in is whether the token is valid. A new user needs nothing from an admin to start: they can approve avatars at once, into the shared space in step 3 and, from step 4a, into their personal swarm, which exists from the first sign-in.

### 8.5 Providers, behind one contract

This section is about the human surfaces and the page. Avatars' clients talk to kkt's own authorization server (8.3), which supports CIMD and dynamic registration whatever the org's server does. A new crate, `kkt-oidc`, owns the human side: two contracts; the generic implementation behind its `real` feature, wrapping the OpenID Connect and JWT libraries so that none of their types leave the crate; and a fake `TokenVerifier` behind `testing` (PRINCIPLES.md rule 8). `TokenVerifier`, given a bearer token and the endpoint it was presented at, returns a verified identity or says the token is not valid for kkt: issuer, subject, the resource the token is for, its scopes, its expiry, the email and whether the provider attests it verified, the org claim if any, and whether a human signed in or an application acted alone (an application-only grant is refused everywhere). `PageSignIn` starts the web page's sign-in, finishes it from the callback, and returns the same identity. One generic OpenID Connect implementation serves every provider; the differences are configuration. `kkt-oidc` is login mode's part of admission, reached through `kkt-admission`'s contract (8.9).

| Authorization server | MCP clients identify by | Subject claim | Notes |
|---|---|---|---|
| Entra ID | Per-client app registration in the tenant | `oid` | Tenant-exact issuer, Application ID URI = kkt URL, v2 tokens. Its email claim is not verified and never admits anyone. |
| Okta | Pre-registered, or dynamic registration with an API token | `sub` | A kkt audience needs a custom authorization server, which is the API Access Management add-on; the org authorization server cannot mint tokens for kkt. Documented, not open. |
| WorkOS | Client ID Metadata Documents, so Claude and ChatGPT connect by URL | `sub` | Google, Apple, email code and magic link for individuals; per-organization SSO by domain; `org_id` claim. CIMD and per-resource audiences are documented. Its default token claims do not include a verified email; the verification claim is configured there and tested (section 13). |
| Auth0, Keycloak, other brokers | CIMD or dynamic registration | `sub` | Same code path; not supported in the first version. |
| Mock | Pre-registered, and a client ID that is a URL | `sub` | 8.6. |

**The identity promise is narrow.** OpenID Connect allows a subject that differs per client (pairwise), so nothing guarantees that the page's id_token and the two human resources' access tokens name the same person. kkt supports a provider only where a documented claim is stable across the page client and the two resources: `oid` on Entra, `sub` on WorkOS and on Okta's custom authorization server. A mismatch is a refusal, never repaired by linking on email. Each supported provider is tested with the same human through the page and both human endpoints.

Configuration, all in the server's config file, gathered by `kkt-settings` and checked by `kkt-config`: public address, issuer URL, the page's client ID and secret, the subject claim, the email claim and the claim that attests it verified (absent for Entra), the org claim, the allowed signing algorithms, the first server admin (8.7). Discovery, signing keys and endpoints come from the issuer's discovery document, cached, refreshed on an unknown key ID at most once per interval set in `kkt-config`, so a stream of bad tokens cannot make kkt hammer the issuer.

### 8.6 The mock authorization server

A dev-only crate, `kkt-mock-issuer`, which the `kkt` program never depends on, so it is never in the release (as with `kkt-scratch`; Sam, 2026-11-05): a small, deterministic OpenID Connect provider and authorization server (discovery document, a sign-in page that lets you type any email and pick claim values, token endpoint, a signing key made at start, pre-registered clients and acceptance of a client ID that is a URL). The end-to-end tests start it in-process on a free port through `tests/common` and point the server's issuer at it. Rule and admission tests use `kkt-oidc`'s fake `TokenVerifier`. Nothing in the `kkt` binary knows it exists. The mock proves kkt's own behavior; it cannot prove compatibility with a real provider, which is what the fixtures and the smoke test in step 3 are for.

### 8.7 Tokens, sessions, modes

- **Bearer tokens** on `/mcp-user` and `/mcp-admin` are the org's authorization server's JWTs, verified on every request against cached keys; kkt stores none. The check: signature by a key of the configured issuer with an allowed algorithm, `iss`, `aud` equal to this endpoint's resource, or the one shared audience where the server has no other (8.3), `exp` not passed, `nbf` not in the future, the subject and any other required claim present, and the token an access token, not an id_token or another kind. Lifetime and refresh are the authorization server's business.
- **Avatar tokens** on `/mcp` in login mode are kkt's own: opaque references to the grant, minutes long (`avatar_access_token_lifetime`), refreshed through rotated refresh tokens with reuse detection until the grant's deadline (`avatar_grant_lifetime`), checked against current state on every request, refused everywhere but `/mcp`. In key mode avatar tokens stay as today, passed as `token`. Every call acts as one avatar.
- **Revocation.** Deactivating or removing a user, suspending an org, removing a role, deactivating an avatar or revoking a grant takes effect on the next request and ends the open waits and feeds concerned, because state is checked every time and the feeds are woken (section 4). This is the direct disable. Removal at the org's authorization server takes effect on the human surfaces when the access token expires, on the web page when its session ends, and for the human's avatars at kkt's next background recheck of their provider session: kkt refreshes the recheck credential every `identity_recheck_interval` (default 1 hour), a definitive refusal suspends the user's avatar grants, and when no recheck has succeeded for `identity_staleness_max` (default 24 hours) they are suspended until one does. A provider that gives the page no refresh token offers no recheck between sign-ins; there the bound is the grant deadline. Replacing an avatar's token by `reclaim`, or its grant by a new approval, ends waits on the old one.
- **Avatar grants** are stored with their tokens hashed; limits on access token lifetime, grant lifetime, pending approvals per user, approvals per minute, the count and lifetime of the permission to create avatars, and unauthenticated requests per minute are `Limits` fields in `kkt-config`. Login mode only.
- **The page sign-in** is a transaction: short-lived, single use, bound to the browser that started it by a cookie of its own, with `state`, `nonce` and PKCE, a fixed callback, and a return target that is only a path on kkt's own page. The id_token's audience must be the page's client ID, and the page's sign-in verification is separate code from the endpoints' access-token verification. The callback is a cross-site navigation, so the transaction cookie is `SameSite=Lax`; the session cookie stays `SameSite=Strict`, and the callback answers with a page on kkt's origin that sends the browser on, so the first page view carries it. A replayed callback, a wrong state or nonce, or an expired transaction is refused.
- **Web page session:** a cookie started by the page sign-in; its length a limit in `kkt-config`, default today's 12 hours. It is kkt's own revocation window: it ends at its lifetime, at sign-out, and at the changes listed under revocation, and it outlives the provider's access token by design. The documentation says so.
- In login mode `kkt register` and `kkt reclaim` run the device grant (8.3) and write the avatar's tokens to the token file, which `kkt watch` reads and refreshes. There is no `kkt login`: the `kkt` command never holds a human credential. A user agent on the command line signs in to `/mcp-user` with its own client's login against the org's authorization server, and the documentation says not to do that on a machine where that user's workers run under the same account.
- **Login mode is a server setting, and a server is in one mode.** In key mode nothing changes: access key, web key, admin key, one user with every role. In login mode the three keys are replaced by sign-in, with no break-glass key.
- **The first server admin** is named in the server's config file, written by `kkt setup` (`kkt-setup` decides the line; the program writes it), readable only by the account the server runs as: by issuer and subject, or by email where the configured verification claim attests it (never on Entra, whose email claim is not verified). The line is a bootstrap: the first sign-in that matches gets the server role and the org admin role of the one org, the bootstrap is consumed in that transaction, and the role is bound to that (issuer, subject). After that the line grants nothing: a later subject with the same email gets no role, two first sign-ins racing get one admin, and removing the role is not undone by a restart. If every server admin is locked out, whoever has root on the host writes a new bootstrap line and restarts, and it is consumed once in the same way. Turning login mode on for an existing server: the migrated user becomes the bootstrap's target, so the first matching sign-in takes over that user, and everything its avatars did stays theirs.

### 8.8 Clients and exposure

For avatars, every client identifies itself to kkt's own authorization server, by CIMD or dynamic registration, and the rows below say only where its callback is. For the human surfaces the client identifies itself to the org's authorization server as the row says.

| Client | Client identification | Callback |
|---|---|---|
| Claude remote connectors (Claude.ai, Claude Desktop, mobile, Cowork) | CIMD (recommended), dynamic registration, or an admin-entered client ID | Anthropic's hosted callback; one connector system for all. Local or desktop integrations that are not remote connectors are not covered. |
| Claude Code | CIMD discovered automatically (avatars only; it has no human surface) | Loopback, `--callback-port` |
| ChatGPT web and desktop | CIMD preferred, dynamic registration once per connector, or a predefined client | A stable callback when the authorization server supports issuer identification (RFC 9207); otherwise a per-connector URL with a callback ID. Use the exact value ChatGPT's connection management shows, not a template. |
| Codex, Cursor, VS Code | Dynamic registration or pre-registered | Loopback; to re-check against current versions before building |
| `kkt` command | A client of kkt's own authorization server, by the device grant (avatars only) | None |

- TLS in front: a reverse proxy, or Tailscale as today. kkt speaks plain HTTP behind it and learns its public address from configuration, since the resource metadata states it and tokens are bound to it. The Host check (`kkt-hosts`) stays; the public name is added.
- Claude.ai and ChatGPT reach kkt from their own servers, so a server they use must be on the public internet. A Tailscale-only server works with Claude Code and the `kkt` command only.
- **Rate limits.** Today's limits stay as they are and keep their names: `sends_per_minute` is per avatar (each avatar its own bucket), `registrations_per_minute` per server. Login mode adds separate `Limits` fields, each with its default written out: per-user sends, active avatars, pending approvals and approvals per minute, the grant and token lifetimes of 8.7, and one for unauthenticated requests (device code requests, wrong codes, bad tokens). They are fields of `Limits` in `kkt-config`, not constants; the config file loads them through `kkt-settings`, and `kkt-provenance`'s `settings_text` records them. Key mode never applies the per-user ones.
- SQLite is fine at this scale.

### 8.9 Admission: one contract, three modes

`kkt-admission` is the one place that owns how callers get in (Sam, 2026-11-06). It has three modes, each a parallel way of doing the same job: open (a keyless loopback install, as today), shared key (key mode: the access, web and admin keys, as today) and OpenID Connect (login mode). A server runs one mode (8.7). `kkt-admission` defines, in one place, what every mode must provide:

- **Admit a caller.** Turn the credential a request presents into who was admitted, a `Principal`, or a refusal. Open admits every caller as the one user. A shared key admits its holder as the one user. OpenID Connect verifies the bearer token (8.7) and names the signed-in user; on `/mcp` it passes an avatar token on without judging it, and `kkt-rules` checks the grant (step 3).
- **Start and end a web session.** A mode starts a session through a sign-in of one of two shapes: a submitted form (shared key: the viewer types the web key, as today) or a redirect and callback (OpenID Connect: the browser is sent to the provider, and the sign-in is finished from the callback, 8.7). Either way the result is a session, and sign-out ends it. Open needs no sign-in.

The shared-key mode stays inside `kkt-admission`: it is small and wraps no library. A mode that wraps a library provides its part from its own crate through that contract. OpenID Connect comes from `kkt-oidc`, whose `TokenVerifier` and `PageSignIn` supply what the contract asks for. `kkt-admission` states what it needs in its own terms and does not depend on `kkt-oidc`; `kkt-server` connects them, as it connects `kkt-rules` to `kkt-web`. A later mode would be added the same way.

**One contract for the page.** `kkt-web`'s `Gate` covers both sign-in shapes, so the pages see one contract whatever the mode. `Gate` says which shape the server uses: none, a form or a redirect. For a form, `Gate` checks what the viewer typed, as today. For a redirect, `Gate` gives the address to send the browser to, with the transaction cookie that binds the sign-in to that browser (8.7), and finishes the sign-in from the callback's query and that cookie. Both shapes end in the same result: a session started, with its lifetime, or a refusal with its reason. The session check, sign-out, the viewer of step 2 and form tokens are the same in every mode. The pages choose between a login form and a sign-in link from the shape alone. `kkt-server`'s `Gate` passes each call to `kkt-admission`, which hands it to the mode; the preview's `Gate` simulates either shape.

## 9. What changes for agents on `/mcp`

| Tool | Change |
|---|---|
| `register` | Key mode as today. Refused in login mode: an avatar is created when the human approves its grant (8.3), or by `create_avatar`. Handles unique within the org, including deactivated and unregistered ones. |
| `create_avatar` | New, login mode only. Refused without the permission to create avatars on the caller's grant; makes one avatar in the caller's swarm, owned by the caller's owner, with the `mcp` permission only, and returns its credentials once. |
| `create_channel` | Unchanged. The channel is in your swarm. |
| `list_channels`, `list_members` | Each channel in a shared swarm gains `swarm`. Home channels unchanged. |
| `list_agents`, `whoami` | Active avatars in your swarm; `whoami` names your swarm when shared, and in login mode shows your grant's permissions and deadline. |
| `reclaim` | Key mode as today: handle and recovery code. Refused in login mode: a new client picks up an existing avatar by the human's approval of a new grant for it. Gains `org_announcement` and `user_announcement` beside the server announcement, lane and memory it returns today. |
| `register`, `whoami` | Gain `org_announcement` and `user_announcement`. |
| `list_lanes` | Takes no token, as today; in login mode it lists the grant's org's lanes. |
| `initialize`, `discover` | Return the announcements before registration, as today; in login mode the grant's user's. |
| Every other tool | Bound to the avatar named by `token`; in login mode to the grant's avatar, and `token`, if given, must name it. |
| Any tool, deactivated | Token refused, as for a revoked agent today. Other avatars get `not_found` for it. |
| Channel notices | From `server` instead of `operator`; actor named by avatar, never by human name. |

On `/mcp-admin`, `create_channel` gains an optional `swarm`. In key mode it defaults to the one swarm. In login mode it is required when the org has more than one swarm, and a call without it is refused with a message saying so; the swarm must be in the admin's org, and every initial member in that swarm. Naming a swarm in one's own org is choosing a target; naming one in another org is `not_found`.

Key mode behavior is unchanged throughout. Each step that touches a tool updates `crates/kkt-mcp/agent-instructions.md` and `admin-instructions.md`, the tool text files (`agent-tools.toml`, `admin-tools.toml`) and both contract snapshots in `tests/contract/`; step 6 adds a third set for `/mcp-user`.

On a single-user server in key mode with no shared swarms, nothing differs from today except the notice sender, two null fields, and revoked agents no longer listed.

## 10. What changes for humans

- **User pages:** my avatars, each with its swarm, channels, memory and a deactivate button; every channel in every swarm I have access to, closed ones included; my swarms with their avatars, channels and groups; create, share, deactivate a swarm; approve a client's sign-in or a device code, choosing the swarm and, if wanted, the permission to create avatars with its count and expiry; see and revoke my avatars' grants; change a channel through one of my avatars in it; my announcement.
- **Org admin pages:** users (add, remove, org roles, invitations), groups, every swarm in the org with a change-owner action, lanes, org announcement, every avatar grant in the org with a revoke action.
- **Server admin pages:** orgs (create, suspend, first admin, claim binding), server roles, server-wide login settings, limits, upgrade history, everything the page has today.

## 11. Existing installs keep working

The migration happens across the steps of section 12, each creating its part. Key mode is unchanged throughout, apart from the three changes below, all made in step 1.

- **Step 1:** every avatar and channel gets its internal ID, and an org and a swarm column holding one default value each, ownerless operator channels included. Revoked agents are deactivated.
- **Step 2:** one user, in the default org, holding all three roles and owning every avatar. The access key signs in as that user; a keyless loopback install behaves exactly like key mode with that one user; the web key is that user on the web page; the admin key keeps opening `/mcp-admin` with every role.
- **Step 3:** nothing. Login mode is a new optional setting. When it is turned on, the migrated user is the bootstrap's target (8.7), so the first sign-in that matches the configured issuer and subject, or verified email, takes over that user, and every old avatar and channel is theirs.
- **Step 4a:** the default swarm becomes that user's personal swarm, which every avatar and channel is already in. No shared swarm is created.
- **Step 5:** the default value becomes the one org, bound to the issuer in login mode, holding today's lanes, retired lanes and pinned versions. Today's announcement stays the server announcement.

Setup, Docker, service files, keys, client configs and CLI flags are untouched.

What an existing install sees change, and nothing else:

1. Revoked agents become deactivated avatars: they leave `list_agents` and their channels, and are `not_found` to other agents. Their messages stay. The migration posts no notices for them. Sam approved this on 2026-11-01 under principle 3.
2. Channel notices come from `server` instead of `operator`, approved in #224.
3. `register` and `whoami` gain two fields, null until an org or user announcement is first saved (steps 5 and 6).

`hide_agent`, the stale cleanup and `unregister` keep today's behavior, including the refusal to hide an agent that owns a channel.

Tests: a populated 0.9.0 database (schema 0019, the same as 0.8.0's) (revoked owners, hidden channels, ownerless channels, notes, memory, lanes with retired ones and pinned versions, old token and recovery hashes, old message IDs) opened by the new version keeps every row and answers every read the same except the three listed changes, each asserted explicitly. Moving references from handles to internal IDs checks the full-text index, the foreign keys and the authors of old messages. Each step's migration, a new numbered file in `crates/kkt-store/migrations/` from 0020 on, is tested in `tests/migrations.rs` from the previous step's schema, so an install upgraded through each step in turn arrives where one upgraded straight to the last does. Two migrated avatars keep two independent send budgets, and a config file with today's limit settings loads unchanged.

## 12. Order of work

The work comes in small steps, so that people using kkt today can try each part without taking on the whole. Each step ships and is useful on its own; a server at any step is complete for the users it admits. Steps 1 and 2 change nothing anyone sees. From step 3 login mode is an optional setting, and key mode is unchanged throughout (section 11). Lanes and memory shipped in v0.8.0 and are per server; step 5 moves lanes under the org. Cross-org sharing is not a step (section 14). Section 12a says which crates each step touches.

- **1. Internal IDs, invisible.** Every avatar gets a stable internal ID, and every reference to an avatar moves from the handle to it, including in-memory keys for waits and rate limits. Avatar and channel rows gain an org column and a swarm column, each holding one default value, so later steps add behavior without reworking old data; `(org, handle)` unique, case-insensitively. Avatars gain an active flag, and revoked agents are deactivated. The three changes of section 11 are made here: revoked agents deactivated, the notice sender renamed to `server`, and the two announcement fields added to `register` and `whoami`, null until steps 5 and 6. Nothing else is built ahead, and no other tool output changes. Tests: the populated 0.9.0 database migrates with the three listed changes, each asserted; the full-text index, the foreign keys and the authors of old messages follow the move to internal IDs; every rule test passes unchanged, except those that expect a revoked agent to still be listed or still in its channels, which change to expect a deactivated one; two migrated avatars keep two independent send budgets.
- **2. Roles, invisible.** Users and roles in the schema; the migration creates one user, in the default org, holding all three roles and owning every avatar. `roles::allows` and the three scope-bound capabilities replace every "is this the operator" check, on `/mcp`, `/mcp-admin` and the web page; authorization inside the write transaction. Who the caller is now travels from admission to the rules on every request, moved here from step 3 so that step 3 adds a new kind of caller rather than the path for one: `kkt-admission`'s `Principal` names who was admitted; `kkt-server`'s `admit` keeps it, where today it drops it; `kkt-mcp`'s two endpoints and `kkt-web`'s `Gate` pass it on; and `kkt-server`'s `RulesSource` and `kkt-mcp`'s `AdminServer` act for one caller instead of being shared by every caller. In key mode every principal is the one user. The step lands as three pull requests, each mostly in one crate (PRINCIPLES.md rule 9): the schema, `roles` and their rule tests (`kkt-store`, `kkt-rules`); the principal and the web contract's viewer (`kkt-admission`, `kkt-web`); then the wiring (`kkt-server`, `kkt-mcp`). Key mode behaves exactly as today: the access key, a keyless loopback install, the web key and the admin key each act as that one user. Tests: every existing test passes unchanged; rule tests with a second user seeded, holding only the user role, that it is refused every power of today's operator, on writes as well as reads.
- **3. Login: OAuth and grants, in one shared space.** Login mode is an optional setting; key-mode installs see nothing new. This step is for a trusted team, not for untrusted users: users are not separated from each other. Every user's avatars live in one shared space, as on today's server: open channels visible to all, closed channels private to their members. The `kkt-oidc` crate with `TokenVerifier` and `PageSignIn`, and the token profile and page sign-in transaction of 8.7; the `kkt-mock-issuer` crate and its use in `tests/common`; protected resource metadata and the 401 challenge on `/mcp`, `/mcp-admin` and the page. kkt's own authorization server for avatars (8.3), in the new crate `kkt-avatar-auth`, from a maintained library or a standard sidecar: metadata, PKCE, dynamic registration and CIMD, RFC 9207, the authorization code grant with the consent page, the device grant with the approval page, opaque access tokens, rotated refresh tokens with reuse detection, the grant store, and the per-request state check; the recheck of the human's provider session and its two bounds. The consent and approval pages pick or create the avatar only; there is no swarm to choose yet. Admission gains its OpenID Connect mode (8.9) and with it the bearer-token caller, carried to the rules by step 2's path: a signed-in user on the human endpoints and the page, and on `/mcp` an avatar token, which `kkt-admission` passes on without judging, since it must not know about agent tokens; `kkt-rules` checks the grant, as `auth.rs` checks a key-mode token today. Every existing tool and page checks a role through `allows`: a server admin, an org admin and an ordinary user each get exactly their powers, so a non-admin sign-in inherits nothing of today's operator. Users created at first sign-in; one org, bound to the issuer; the bootstrap server admin, who is also the org admin. The inventory of section 5 done and every read and event in it scoped to the caller's capability; subscriptions for waits and the web feed carry the subscriber, decide per event in `kkt-rules`, and end when the subscriber loses the right; a per-user web `Source`, showing a user the open channels and the closed channels one of their avatars is in. Avatar details split from memory, which only the owner and org admins read. Deactivation as one operation, beside `hide_agent` and the stale cleanup, which keep their rule. Human channel changes through one avatar. The administrative actor record. The login-mode `Limits` fields, unused in key mode. Minimal user pages: my avatars and their grants, the consent and approval pages, revoke, deactivate, sign out; and the org admin's user list with remove and roles. Revocation live: waits, feeds and sessions end on deactivation, grant revocation or replacement, reclaim, sign-out, role removal, user removal and org suspension. `kkt register` and `kkt reclaim` by the device grant; every `/mcp` call bound to the grant's avatar; `register` and recovery-code `reclaim` refused in login mode; `create_avatar` under the permission to create avatars, shown by `whoami`. When this step lands, `CLAUDE.md`'s crate table gains `kkt-oidc`, `kkt-avatar-auth` and `kkt-mock-issuer`, and its axum rule names `kkt-avatar-auth` as a fourth crate that may use axum (Sam, 2026-11-05). Tests, through HTTP with the mock, not only seeded rule calls: three roles each confined to their powers on every tool and page; a test per capability that it cannot reach the next scope, on writes as well as reads; two signed-in users unable to act on each other's avatars or grants, or to read each other's avatars' memory or closed channels only the other's avatars are in, checking the complete serialized results; a human token with the wrong audience, issuer, expiry, algorithm, kind or a future `nbf` refused, and an application-only token anywhere; an avatar token at `/mcp-admin`, the web page and the token endpoint refused; a replayed or mismatched callback refused; each revocation ending an open wait and an open feed with no new message arriving; a device code expired or already used refused, with nothing created; a reused refresh token revoking the grant; `create_avatar` from a grant without the permission, from a child, past the count, and after the parent's revocation refused, and the count raced; a revoked parent's children refused; a second grant for an avatar ending the first's wait; the human disabled at the provider ending the avatars' grants within the recheck bound; a slow subscriber's lag report carries no count. Gate: captured, sanitized claim fixtures from each supported provider's page and `/mcp-admin`, and one login and refresh against a real supported provider; one native login of each client in 8.8 against kkt's authorization server.
- **4a. Personal swarms.** Swarms in the schema (ID, owner, personal and active flags). Every user gets a personal swarm, made with the user; the migration makes one for each existing user. The default swarm of step 1 becomes, if the org has one user, that user's personal swarm (always so in key mode, where nothing visible changes); otherwise the shared space of step 3, owned by the first server admin, to which every user of the org has access. `decide` takes the swarm test and the visibility rule is live; every read and event in the inventory is scoped to the swarm. The consent and approval pages choose the swarm, from the swarms the user has access to. Every user sees every channel in their swarms, closed ones included. Human channel creation takes a swarm; admin `create_channel` gains `swarm`. Tests: a two-user test, seeded directly, that neither sees the other's channels, agents, events or waits, checking the complete serialized results; a device code approved by a user without access to the chosen swarm refused, with nothing created; a step-3 database with one user and one with several each migrated as described.
- **4b. Shared swarms and groups.** Groups; shared swarms created, shared, deactivated and reassigned on the web page; the migration shares step 3's shared space with a group of every user who had access to it. Grants into shared swarms, the approval pages offering every swarm the user has access to; the access rule and the transition table live, including the cascade and the refusal of pending approvals on lost access; every user with access seeing every channel in a swarm. The swarm pages of section 10. Org admin tools for groups. Tests: each row of the transition table, with a user removed during an open stream, during a wait, and concurrently with a write; loss of the last grant and retention of another grant as separate cases; a reassignment that collides with the new owner's swarm name, and one to a user of another org, refused.
- **5. Orgs.** Several orgs on one server. Org admission by claim binding and by invitation; the per-request org match; the disabled user row; same-org checks on every relationship write. Lanes scoped to the org with existing lanes, retired lanes and pinned versions in the default org; org announcement tools under new names. `kkt lanes` and `kkt agent revoke` take `--org`, defaulting to the only org (the program's `cli.rs` and its `lanes` and `agent` commands). Server admin pages for orgs, claim bindings and server roles; the server announcement tools need the server role in login mode. Tests: equal handles and equal lane names in two orgs through memory, reclaim, ownership, waits and administration; a token admitted by claim, by invitation, and refused by neither; a known subject presenting a token for another org refused and not moved; a removed user's token refused; a duplicate claim binding refused; an org admin unable to bind a claim or grant the server role, or to reach another org, on writes as well as reads.
- **6. The user surface, optional.** Any time after step 4. `/mcp-user`, a third endpoint in `kkt-mcp` with its own tool text, instructions and contract snapshot: list my swarms, list my avatars and their grants, revoke a grant, deactivate, read their channels and memory, the user announcement, also set on the page. No approval here: that stays on the page. Tests: every read, revocation and deactivation on `/mcp-user` owner-only, including another user's grant refused and changing nothing; no tool on `/mcp-user` approves a pending request; a `/mcp-user` token refused at `/mcp-admin` and the reverse where the server distinguishes them; an avatar token at `/mcp-user` refused. Gate: claim fixtures from each supported provider's `/mcp-user` resource.

## 12a. Where the work lands

kkt is a workspace of crates. `CLAUDE.md` lists each crate and what it owns, and `PRINCIPLES.md` gives the rules for splitting code among them. This plan adds three crates and grows existing ones. Each new third-party library is wrapped in one crate, none of its types leave that crate, and its features are added to `.github/third-party-features.txt`.

**New crates**

| Crate | Owns | Must not | Features |
|---|---|---|---|
| `kkt-oidc` | The OpenID Connect client, the human side of login (8.5, 8.7), which supplies login mode to `kkt-admission`'s contract (8.9): `TokenVerifier`, `PageSignIn`, the page sign-in transaction, the issuer's discovery document and signing-key cache. | Know about avatars, grants or roles; decide who belongs to an org (that is `kkt-rules`). | `real`: the OpenID Connect implementation, which fetches discovery and keys. `testing`: a fake `TokenVerifier`. |
| `kkt-avatar-auth` | kkt's own authorization server for avatars (8.3): its metadata, PKCE, client identification (CIMD, dynamic registration), RFC 9207, the authorization code and device grants, the token endpoint, opaque tokens and refresh rotation. Uses axum, as the fourth crate allowed to (Sam, 2026-11-05). | Decide who may approve what or what a grant allows, or store grants. It states what it needs in a contract of its own, which `kkt-server` adapts to `kkt-rules` and `kkt-store`, as it adapts them to `kkt-web`'s `Source`. | Set with the library or sidecar it wraps (section 13). |
| `kkt-mock-issuer` | The mock authorization server of 8.6, started in-process by `tests/common` (Sam, 2026-11-05). | Be anything but a dev-dependency. | none |

The consent and approval pages are human pages, so they are in `kkt-web`, not `kkt-avatar-auth`.

**Existing crates and the program, by step**

| Step | Where it lands |
|---|---|
| 1. Internal IDs | `kkt-store`: the migration, and every reference by internal ID. `kkt-rules`: the send limiter (`rate.rs`) and open waits (`waiting.rs`) keyed by internal ID instead of handle; the notice sender `server`. `kkt-events`: notices carry the internal ID. `kkt-mcp`: the two announcement fields, in the tool text and contract snapshots. Output keeps handles. If the internal ID is a row ID rather than a drawn one, `kkt-names` does not change. |
| 2. Roles | `kkt-store`: users and roles. `kkt-rules`: the `roles` module. `kkt-admission`: `Principal` names who was admitted. `kkt-web`: `Gate` returns a viewer. `kkt-server`: keeps the principal in `admit`, and builds `RulesSource` per caller. `kkt-mcp`: both endpoints act for one caller. |
| 3. Login | The three new crates. `kkt-admission`: the mode contract of 8.9 and the OpenID Connect mode, stating in its own terms what it needs from a token verifier and a redirect sign-in, which `kkt-server` adapts to `kkt-oidc`; the 401 challenge and the protected resource metadata, which `kkt-server` serves. `kkt-rules`: grants and their cascade, deactivation, the actor record, human changes through one avatar, per-event visibility. `kkt-store`: grants, users' (issuer, subject), the actor record. `kkt-web`: sign-in by redirect through `Gate` (8.9), consent, approval and user pages, and the per-user `Source`. `kkt-mcp`: `create_avatar`, and binding every call to the grant's avatar. `kkt-settings` and `kkt-config`: login settings and the new `Limits` fields, recorded by `kkt-provenance`. `kkt-hosts`: the public name. `kkt-agent-client` and the program's `agent` command: the device grant. `src/machine/credentials.rs`: token files that hold refreshable tokens, refreshed by `kkt-watch` through its injected server; today's token files keep loading. `kkt-setup`: the bootstrap line, which the program writes. |
| 4a, 4b. Swarms | `kkt-rules`: `access::decide`'s swarm input, access to a swarm, and the transitions. `kkt-store`: swarms and groups. `kkt-format`: never names the personal swarm. `kkt-mcp`: admin `create_channel`'s `swarm`. `kkt-web`: swarm and group pages. |
| 5. Orgs | `kkt-rules` and `kkt-store`: orgs, admission by claim and invitation, same-org checks, lanes per org. The program's `cli.rs` and its `lanes` and `agent` commands: `--org`. `kkt-web`: server admin pages. `kkt-lane-files` does not change: a lane file names no org. |
| 6. `/mcp-user` | `kkt-mcp`: a third endpoint with its own `user-tools.toml`, `user-instructions.md` and `tests/contract/user-tools.json`. `kkt-server`: mounts it. |

Steps 1 and 3 each touch many crates. Each lands as several pull requests, each mostly in one crate (PRINCIPLES.md rule 9), in an order where every pull request builds on a contract already merged. Step 2 is split that way in its own entry.

**`CLAUDE.md` changes as the steps land:** in step 2, the `kkt-admission` row names the principal it hands on; in step 3, rows for the three new crates, the `kkt-admission` row naming its three modes and the contract each provides, and the axum rule widened to `kkt-avatar-auth`; in step 6, the `/mcp-user` tool text and instructions under "Other files". The fourth design principle of section 2 is added whenever Sam adopts it.

## 13. To verify before building

- Points 1 and 2 of the 2026-11-01 review are closed by `agent-credentials-design.md`. Point 1, whether a client's refresh token at the org's authorization server can obtain a token for another audience: it can, and the design no longer depends on it, because no avatar token comes from that server; the only tokens it issues for kkt are for the human surfaces, held by the human's own client. Point 2, whether Entra gives distinguishable audiences: it does not for one app registration, and the design no longer depends on it; where the two human endpoints share one audience the role check separates them (8.3).
- Which maintained OAuth 2.1 library or sidecar runs kkt's authorization server, and that it does the device grant, dynamic registration, CIMD, RFC 9207 and refresh rotation with reuse detection. Still open (Sam, 2026-11-05). The answer sets the size of `kkt-avatar-auth`: with a sidecar it holds little more than the glue to the consent and approval pages and the sidecar's configuration.
- That each client in 8.8 completes its native login against kkt's authorization server with the consent page, and how a worker's MCP client takes an avatar created by `create_avatar`; the `kkt token` headers helper is the recorded fallback (`agent-credentials-design.md`, section 9).
- Which providers issue the page client a refresh token for the background recheck, and each one's observed delay between disabling a user and refusing the refresh.
- WorkOS: which claim attests a verified email, and that it is present on access tokens for the configured resources; that `org_id` is present for organization sign-ins; that its CIMD support accepts Claude's and ChatGPT's metadata documents.
- ChatGPT with Entra direct: whether Entra supports issuer identification, which decides between the stable callback and the per-connector one; Entra matches redirect URIs exactly.
- Codex, Cursor and VS Code client behavior against current versions, as `docs/waiting.md` was checked.

## 14. Not in this design

Rejected or deferred, so they are not re-proposed:

- kkt as the authorization server for humans (sign-in, human tokens, a consent screen for human power). A broker does it better. kkt's own authorization server issues avatar tokens only (8.3).
- Mint codes redeemed by `register`, `kkt login`, and recovery-code `reclaim` in login mode: replaced by avatar grants.
- Sender-constrained tokens (DPoP), RFC 8693 token exchange, a local signing bridge, and a `kkt token` headers helper: deferred, with the notes in `agent-credentials-design.md`, section 12.
- The permission to create avatars as anything but a grant permission the human sets: not a lane, not a role, not a tool's input.
- Agents creating swarms, or a user's agent creating and sharing them from `/mcp-user`.
- Avatars in several swarms, moving between swarms, or choosing their swarm at `register` (`list_swarms`, `swarm` parameter).
- Self-registration on `/mcp` in login mode with the sign-in alone.
- Reversible deactivation; returning avatars home on removal; moving avatars between users.
- Deactivating a user's shared swarms when the user is removed.
- The role-to-permission mapping as data; custom roles.
- A "revoked but visible" avatar state kept after migration.
- The web key as break-glass in login mode; key mode and login mode on one server; the mock behind a `kkt serve` flag.
- Linking two subjects to one user, by email or otherwise.
- A general audit log.
- Server-wide lanes; moving today's announcement to the org level.
- Cross-org sharing: a design of its own, when there is a need. The internal avatar ID leaves room for it.
