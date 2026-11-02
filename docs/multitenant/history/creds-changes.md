# What changed, 2026-11-01

Editor: petite-dragonfly. Sam's decision: adopt the Fable review's simpler design, no DPoP and no local bridge now.

## agent-credentials-design.md

Rewritten from a proposal with research history into the settled design.

- **kkt runs a standard OAuth 2.1 authorization server in login mode, for avatar tokens only**, from a maintained library or a standard sidecar. Humans still sign in through the org's provider; kkt is a confidential client and provider tokens stay on the server.
- **Ordinary grants replace RFC 8693 token exchange**: authorization code with PKCE for clients with a browser (consent page picks swarm and avatar), device authorization grant for headless workers through `kkt register` and `kkt reclaim`.
- **One grant record per avatar** with owner, org, swarm, avatar, permissions, deadline, revocation state, parent grant. Opaque minutes-long access tokens, rotated refresh tokens with reuse detection, audience `/mcp` only, per-request state check, cascading revocation that ends open waits.
- **The permission to create avatars** (`avatars:create`), per Sam's note via tawny-ocelot: a grant permission set only by the human at approval, with count and expiry, confined to the avatar's own swarm, never passed on, children revoked with it, visible in `whoami`, changed by no tool. Named as a general permission, not an orchestrator feature; lanes stay soft.
- **From both reviews**: the approval step's safeguards (code typed by the signed-in human, swarm chosen by the human, agent names advisory, deadlines, rate limits, single use, CSRF and framing protection, deliberate confirmation); honest revocation promises (provider disablement noticed at kkt's next background recheck, with the interval and staleness bounds stated, plus the immediate admin disable); login mode refusing `register` without a grant and recovery-code `reclaim` (astra finding 5); hosted clients holding avatar-only grants and `/mcp-user` as the one hosted place with human power; the same-OS-account limit; what kkt does not build itself.
- **Removed**: the unverified token-splicing claim, the local bridge, DPoP (deferred with Sam's note to revisit at implementation time), token exchange, the research summary. A `kkt token` headers helper is recorded as considered, found awkward, and kept as the fallback.
- Key mode unchanged.

## roadmap.md

- **Section 8 rewritten**: 8.1 two authorization servers (org's for humans, kkt's for avatars); 8.2 the human flow, now for `/mcp-user`; 8.3 avatars by grant, the permission to create avatars, two human endpoints and the Entra shared-audience case, nothing around the grant; 8.5 and 8.7 adjusted (opaque avatar tokens, recheck bounds, grant limits, no `kkt login`); 8.8 clients note.
- **Mint codes, `kkt login`, `register --code`, and the three-audience scheme removed** everywhere they appeared: sections 1, 2, 3, 4 (transitions), 5, 6, 7, 9, 10, 12 and 14.
- **Section 9**: `register` and `reclaim` refused in login mode; new `create_avatar` row; `whoami` shows permissions; every tool bound to the grant's avatar.
- **Step B** rewritten for kkt's authorization server, the grant store, the consent and approval pages, the recheck, and the matching tests and gate. Steps C and D adjusted (no approval on `/mcp-user`, grants into shared swarms).
- **Section 13**: points 1 and 2 closed with the reasons; new items to verify (library or sidecar choice, each client's native login, how a worker's client takes a created avatar, which providers give the page a refresh token).
- **Section 14**: kkt as the humans' authorization server still rejected; mint codes, DPoP, token exchange, the bridge and the helper listed as replaced or deferred; the permission to create avatars as anything but a grant permission rejected.

The roadmap's statements about today's code were checked against `origin/next` (b206f24): `admit` in `app.rs` drops who the credentials belong to; the avatar `token` is read per tool call in `mcp.rs`; `wait_for_messages` authenticates once and ignores non-message notices; `agent_view` and `admin_agent` both return `AgentView`; `overview` bundles server details with channel data; `sends_per_minute`, `registrations_per_minute` and the 12-hour `web_session_lifetime` are in `config.rs`; `kkt register --token-file`, `KKT_TOKEN`, and the `headersHelper` and `http_headers_helper` entries `kkt connect` writes are as described. All held; no roadmap statement about today's code needed correcting.
