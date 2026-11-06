# Agent credentials for multi-tenant kkt

Status: settled design, 2026-11-01, decided by Sam after two reviews. It is the agent-login part of the multi-tenant roadmap (`roadmap.md`, section 8), and closes that roadmap's open points 1 and 2. Nothing here is implemented. Revised on 2026-11-05 to name the crates kkt was split into in 0.9.0; the design is unchanged (`history/crates-changes.md`).

## 1. The problem

kkt is becoming a remote server for many humans. Humans sign in through an outside authorization server: Entra ID, Okta, or a broker such as WorkOS for Google and Apple. Each human runs many AI agents (Claude Code, Codex, Claude.ai, ChatGPT, custom workers), often on one machine under one OS account; an orchestrator in tmux starts worker agents. Each agent acts as an **avatar**: an identity in exactly one **swarm**.

The hard requirement: an agent holds only its avatar's power. No credential on an agent's machine, and no tool an agent can call, lets it act as the human: make avatars outside its swarm, take over a sibling, or use the human's or an admin's tools.

Two things are therefore never done:

- The human's sign-in is never placed on an agent's machine. A refresh token from the outside authorization server can be redeemed for a token to any resource that client and user may use (Entra documents this; RFC 8707 narrows the access token, not the refresh token), and Entra v2 tokens carry the API's client ID as `aud`, so audiences do not separate an agent's power from the human's.
- An avatar is never a plain, unexpiring bearer secret with no tie to the human who approved it.

## 2. The design in short

In login mode kkt runs a standard OAuth 2.1 authorization server, for **avatar tokens only**. Humans still sign in through the outside provider; kkt is a confidential OpenID Connect client of it, and the human's provider tokens stay on the server. An avatar comes from an ordinary OAuth grant approved by the signed-in human on kkt's own page, where the human creates or picks the avatar and, once swarms exist, chooses its swarm. The grant's tokens reach `/mcp` and nothing else. kkt keeps one grant record per avatar and checks it on every request, so revocation is immediate and cascades.

Key mode, today's single-user install, is unchanged.

## 3. Two modes, two authorization servers in login mode

| Surface | Key mode | Login mode: who issues the token |
|---|---|---|
| `/mcp` (avatars) | Access key plus avatar `token`, as today | kkt's own authorization server. Audience `/mcp` only. |
| `/mcp-user`, `/mcp-admin` (a human's agent) | Web key, admin key, as today | The org's authorization server, as roadmap 8.2: kkt is the resource server and verifies the JWT. |
| Web page | Web key | kkt's page sign-in: OpenID Connect to the org's authorization server, confidential client, session cookie. |

Each endpoint publishes its own protected resource metadata naming its authorization server, and answers an unauthenticated request with a 401 challenge, as the MCP spec requires. kkt's authorization server is at kkt's own issuer URL and publishes standard metadata.

The human surfaces are where human power lives. A hosted platform that holds a `/mcp-user` or `/mcp-admin` grant (Claude.ai, ChatGPT) holds human power; that is the one place a hosted platform does, it is the human's deliberate choice to connect such an agent, and the documentation says what it means. Where the org's authorization server cannot give the two human endpoints distinct audiences (Entra v2), the role check alone separates them; that is a choice between two human surfaces, never an agent's escalation.

## 4. How an avatar comes to be

This section describes the end state. Login arrives before swarms (roadmap section 12, steps 3 and 4a): until then every avatar is in the one shared space, the consent and approval pages pick or create the avatar only, and the choice of swarm below is added with swarms.

### 4.1 Clients with a browser: authorization code with PKCE

Claude Code, Codex, Claude.ai and ChatGPT use their native OAuth login against kkt's authorization server: discovery from the `/mcp` resource metadata, client identification by Client ID Metadata Documents or dynamic registration (RFC 7591), PKCE S256, `iss` in the authorization response (RFC 9207). The authorization endpoint is kkt's own page:

1. The human signs in to kkt's page if not already (the ordinary page sign-in through the org's provider).
2. The consent page shows, from server-side state only: who is signed in, the client (its registered name marked as the client's own claim, with its redirect URI), the swarms the human has access to, and the avatars they own that have no live client. The human picks a swarm and either names a new avatar or picks an existing one, and confirms with a button.
3. kkt creates the avatar if new, writes the grant record, and issues the code. The client redeems it for an access token and a refresh token.

An avatar's handle is chosen by the human or by the server (`kkt-names`); a client has no way to supply one here.

### 4.2 Headless workers: the device authorization grant (RFC 8628)

A worker with no browser gets its avatar through the `kkt` command, which runs the device grant against kkt's authorization server and writes the resulting tokens to a private token file (`kkt register --token-file`, which makes a new avatar; `kkt reclaim --handle <h> --token-file`, which asks for an existing one). The command prints the verification URL and a user code. The human, in a browser where they are signed in to kkt, opens the approval page, types the code, sees the request as the server resolved it (the requesting command's suggested name and purpose marked as suggestions, the swarms they may choose), chooses the swarm, and confirms. The command polls the token endpoint at the interval kkt states and receives the tokens.

`kkt watch` and every other `kkt` command that talks to `/mcp` reads that token file and refreshes through it.

### 4.3 The approval step's safeguards

Both pages are pages of `kkt-web`, run on kkt's own origin, and follow the rules of today's web changes (`kkt-web`'s `change.rs`): a form `POST` with the viewer's form token, origin checked, with `Content-Security-Policy: frame-ancestors 'none'` and `X-Frame-Options: DENY` so they cannot be framed.

- The human enters the device code by typing it, while signed in. A link that carries the code may open the page but the code is shown for comparison and the swarm and confirmation are still the human's actions; approval never happens by visiting a link.
- The swarm is chosen by the human from the swarms they have access to now, resolved server-side. Nothing the agent sends names the swarm.
- Agent-supplied text (a suggested handle, a purpose, a client name) is advisory, shown as plain escaped text and marked as the requester's claim. The human may change the handle.
- A pending request binds, at creation and immutably: issuer, org, the requesting client, the suggested name, deadline, and a nonce. Approval adds the human (user ID), the swarm, the avatar and the permissions. It is consumed once, atomically, in the transaction that writes the grant; a second approval or redemption of the same code is refused.
- Device codes and authorization codes expire in minutes (`device_code_lifetime`, `authorization_code_lifetime` in `kkt-config`). Polling faster than the stated interval is answered with `slow_down`.
- Rate limits: pending requests per user, approvals per user per minute, and unauthenticated requests per minute (device code requests, wrong codes, bad tokens), each a `Limits` field with its default written out. A wrong user code counts against the signed-in user who typed it.
- Confirmation is a deliberate action: a button that names the swarm and the avatar, not a default-selected form.

Strong authentication cannot prove the human understood what they approved. The page's job is to make what is approved exactly as plain as it can be.

## 5. The grant

One avatar-grant record per avatar. An avatar that gets a new grant (a human picking it on a consent page, or `kkt reclaim`) has its old grant revoked in the same transaction, which ends that grant's open waits.

| Field | Meaning |
|---|---|
| Grant ID | Stable, internal. |
| Owner | The user, by user ID, which keys on (issuer, subject). |
| Org, swarm, avatar | Fixed at approval. The avatar is in this swarm for good (roadmap section 1); before swarms arrive, the one shared space. |
| Permissions | `mcp` (act as the avatar), and optionally `avatars:create` with a remaining count and its own expiry (section 7). Set by the human at approval; shown to the agent by `whoami`; changed by no tool. |
| Deadline | When the grant stops refreshing, at most `avatar_grant_lifetime` from approval (a `Limits` field, default 30 days). |
| Revocation state | Active, or revoked with when, by whom (user, swarm owner, org admin, server, parent) and why. |
| Parent grant | The grant of the avatar that created this one (section 7), or none. |
| Client | The registered client or the `kkt` command, for the human's pages. |
| Run | The `run_id` of the server run that created it (provenance). |

**Tokens.**

- Access tokens are opaque references to the grant, not JWTs: kkt reads its own state on every request anyway, and there is no second party to verify them. Lifetime `avatar_access_token_lifetime` (default 15 minutes). Audience `/mcp`: the token is refused at `/mcp-user`, `/mcp-admin`, the web page and kkt's own token endpoint as a subject.
- Refresh tokens are rotated on every use, with reuse detection per RFC 9700: presenting a refresh token already rotated away revokes the grant. Refresh stops at the deadline; the human approves again if the avatar should go on.
- Stored hashed, like today's tokens and recovery codes, from the injected `Entropy`.

**Every request checks current state**: the grant is active, the avatar is active, the swarm is active, the owner is an active user with access to the swarm, the org is not suspended. The avatar is the grant's; in login mode the `token` parameter is optional and, if given, must name that avatar. A failed check is refused like a revoked agent's token today.

**Revocation cascades**: revoking a grant revokes its children; deactivating an avatar revokes its grant; deactivating a swarm, removing a user, or suspending an org revokes every grant beneath it. Each revocation wakes the grant's open waits and feeds, which recheck and end (roadmap section 4). Who may revoke: the owner, the swarm's owner, an org admin, and the server on the transitions in the roadmap's table.

## 6. What revocation promises, honestly

- **kkt's own state is immediate.** A revocation or deactivation made on the web page, on `/mcp-user`, on `/mcp-admin` or by `kkt agent revoke` takes effect on the next request and ends open waits. This is the direct disable every admin has, and the one to use when it matters.
- **Disablement at the provider is noticed later.** kkt keeps the human's provider refresh token from their page sign-in on the server (offline access where the provider offers it) and refreshes it in the background every `identity_recheck_interval` (default 1 hour). A definitive refusal (`invalid_grant`) suspends the user's avatar grants and ends their waits. A transient failure (network, 5xx, throttling) is retried; when no recheck has succeeded for `identity_staleness_max` (default 24 hours) the grants are suspended until one does or the human signs in again. So a human disabled at the provider keeps their avatars running for at most one recheck interval, or the staleness bound when the provider is unreachable. A provider that issues the page no refresh token gives kkt no recheck between sign-ins; there the bound is the grant deadline, and the documentation names the provider.
- **Sign-out** ends the page session. It does not revoke avatar grants, which are separate grants the human made; they are revoked on the page. The recheck credential outlives the session; it is readable only by the server.
- **Provider events** (WorkOS events, Entra lifecycle notifications) are not consumed in the first version. They can shorten the bound later without changing the design.

## 7. The permission to create avatars

An avatar that starts other agents needs to create their avatars without a click each. That ability is a **permission on the avatar's grant**, "can create avatars" (`avatars:create`), a hard line that only the human sets, at the one moment they authorize token issuance: the consent page or the device approval page. It is a general permission of any avatar. It is not a lane and has nothing to do with whether the avatar's lane is `orchestrator` or anything else; lanes stay soft and grant nothing.

- The human grants it with a count and an expiry, both capped by `Limits`: "this avatar may create up to N avatars in this swarm until T".
- It is confined to the avatar's own swarm. There is no way to ask for another.
- It is never passed on. An avatar it creates cannot create avatars unless the human grants that avatar the permission directly, on a page.
- Revoking the permission, or the avatar, revokes the children.
- An agent can see its own permissions, in `whoami`, and cannot change them. No tool takes a permission as input.

With it, the avatar creates a child through `/mcp` (a tool, `create_avatar`, refused to grants without the permission). The child gets its own grant record with the creator's as parent, the same swarm, the `mcp` permission only, and a deadline no later than the parent's. Its credentials are returned once. The count is spent atomically.

How the creating agent hands the credentials to the worker's client is the client question in section 9. For a worker driven through `kkt watch` or a custom agent, a token file does it.

## 8. What login mode removes on `/mcp`

Nothing goes around the grant. In login mode:

- `register` without a grant is refused: every connection to `/mcp` already is an avatar's, approved by the human, and the tool answers that avatars are created at sign-in. (`create_avatar` is the only in-band creation, and only under section 7.)
- Recovery codes are not issued, and `reclaim` with a recovery code is refused. Picking up an existing avatar from a new client is the human's approval of a new grant for it (sections 4.1, 4.2), which revokes the old one.
- The avatar is derived from the bearer token only. There is no identity override: a `token` parameter naming another avatar is refused.
- The access key, web key and admin key do not exist; there is no fallback from one mode to the other on one server.

Key mode keeps `register`, `reclaim`, recovery codes and the `token` parameter exactly as today. The tools stay in the contract (`tests/contract/tools.json`); only their behavior in login mode changes, and `crates/kkt-mcp/agent-instructions.md` says so.

## 9. Clients

| Client | Gets its avatar by | Holds |
|---|---|---|
| Claude Code, Codex | Native OAuth login to kkt's authorization server (authorization code, PKCE, loopback redirect), consent on kkt's page | Avatar access and refresh tokens, in the client's own credential store |
| Claude.ai, ChatGPT | The same, through the vendor's hosted callback; client by CIMD or dynamic registration | An avatar-only grant at the vendor. No human power unless the human connects `/mcp-user` or `/mcp-admin` separately |
| `kkt watch`, `kkt register`, `kkt reclaim`, custom workers | The device grant, into a private token file | Avatar tokens in a file only the account can read |
| Avatars created by an avatar with `avatars:create` | `create_avatar` | Whatever the creating agent passes on |

`kkt connect` in login mode writes the client's server entry without a key and leaves the login to the client. A `kkt token` headers helper (the client calling `kkt` for a fresh token on each connection, as `kkt key header` serves the access key today) was considered and found awkward: it puts the credential in the client's process and environment, runs only at connection time, and differs per client. It is recorded here as the fallback for a client that cannot log in natively, to revisit at implementation time; so is a child's credentials reaching a worker's MCP client. Section 11 lists what to check against current client versions.

## 10. Limits, stated plainly

- **Same OS account.** Processes under one OS account can read each other's token files and attach to each other. Siblings on one machine can take each other's avatars; they cannot take the human's power, because no human credential is on that machine: the page session is in the browser, the recheck credential on the server, and the `kkt` command holds avatar grants only. Where sibling isolation matters, run each agent as its own OS user, in a container with its own mounts, or in a VM. The documentation says so; kkt does not claim otherwise.
- **The human's own hosted agents** on `/mcp-user` or `/mcp-admin` act with the human's power, prompt injection included. kkt keeps the approval step off those surfaces for that reason: an agent there can list and revoke grants, never approve one.
- **A copied avatar token** is usable until its minutes run out or the grant is revoked. Tokens are not bound to a key (no DPoP, section 12), so the defense is short lifetimes, rotation with reuse detection, and the per-request state check.

## 11. What kkt does not build itself

- The authorization server: a maintained OAuth 2.1 library inside the server, or a standard sidecar (Keycloak, Ory Hydra or the like) that kkt configures. It supplies metadata, PKCE, dynamic registration, CIMD, RFC 9207, the device grant, refresh rotation and the token endpoint. kkt supplies the consent and approval pages and the grant store behind them. The library, or the glue to the sidecar, lives in its own crate, `kkt-avatar-auth`, which decides nothing about who may approve what: it asks through a contract of its own that `kkt-server` adapts to `kkt-rules` (roadmap, section 12a). Library or sidecar is still open (roadmap, section 13).
- Sign-in with the provider: a standard OpenID Connect relying-party library, wrapped in the `kkt-oidc` crate, which supplies login mode through `kkt-admission`'s contract for every mode (roadmap, 8.5 and 8.9).
- Hashing and randomness: existing code: randomness from `kkt-names`' `Entropy`, token hashing in `kkt-rules`' `auth.rs`.
- No custom token format, grant type, approval protocol or delegation profile. No RFC 8693 token exchange: there is no subject token to exchange, and the parent grant is recorded server-side for the cascade.

The authorization server is still an issuer kkt operates, library or not: budget a security review, signing-key and secret rotation (encrypted at rest), and a recovery procedure for it.

## 12. Deferred, with notes

- **Sender-constrained tokens (DPoP, RFC 9449)**: revisit at implementation time, when client support may have changed. None of Claude Code, Codex, Claude.ai or ChatGPT does DPoP today, and against same-account siblings it buys little. It adds to this design without changing it: a `cnf` on the grant.
- **A local bridge or token helper**: see section 9.
- **An external authorization server issuing avatar grants**: not in the first version; kkt's grant record is the one place avatars come from.
- **ID-JAG, identity chaining, agent grant drafts**: working-group drafts; none supplies the confinement kkt enforces itself.

## 13. Before committing code

Build one complete path with one provider and try to break it: a worker presenting its token at `/mcp-admin`, at `/mcp-user` once it exists, and at the token endpoint; a copied token after revocation; a refresh token reused; `create_avatar` from a child, past the count, after the parent is revoked; approval of a device code by a user without access to the chosen swarm, once swarms exist; a replayed approval; disabling the human at the provider and timing the recheck; revocation during an open wait; a second client picking an avatar that has a live grant. Then check each client in section 9 against its current version.
