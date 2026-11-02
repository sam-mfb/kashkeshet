Fable review of agent-credentials-design.md (2026-11-01), summary as returned.

Fact check: Entra RT and aud claims, RFC 8707, MCP spec (AS may be co-hosted, no passthrough), MCP roadmap (DPoP, delegation pending), DPoP §11 limit all verified. Claude Code and Codex: native OAuth (DCR/CIMD), static headers, headers helpers run per connection and re-run on 401/403; no DPoP (open Claude Code bug confirms). Vendor agent products all differ. ID-JAG is a WG draft (rev 06), identity chaining in RFC Editor queue. NOT verified: the "token-splicing flaw reported to the OAuth WG in 2026"; real 2026 token-exchange bugs are n8n CVE-2026-59208, Open WebUI CVE-2026-70482, Keycloak CVE-2026-97846.

Findings:
1. Hosted clients can hold an avatar: kkt's AS consent page picks/creates the avatar and swarm; Claude.ai/ChatGPT get an avatar-only grant via ordinary auth code + DCR. Only DPoP is lost.
2. RFC 8693 is the wrong tool: no subject token to exchange; use ordinary grants with plain claims (sub=avatar, swarm, owner, aud=/mcp); record parent grant server-side for cascade revocation.
3. Bridge heavier than needed: Claude Code headersHelper / Codex http_headers_helper running `kkt token` gives fresh short-lived tokens, no proxy; interactive users use `claude mcp login` / `codex mcp login`. Today's KKT_TOKEN env is visible to the model. DPoP buys little given same-account siblings; add later when clients support it.
4. Approval spoofable: use device authorization grant (RFC 8628); human enters/compares code while signed in and picks the swarm; agent-chosen name advisory; expire in minutes; rate-limited.
5. Orchestrators: scope avatars:create for its own swarm, count N, expiry T, granted once; children revoked with parent.
6. kkt issuing tokens is accepted MCP practice only as a standard OAuth 2.1 AS from a library or sidecar (Keycloak etc.): metadata, PKCE S256, RFC 9207 iss, DCR and CIMD. External IdP shouldn't issue avatar grants.
7. Revocation lag: provider disable noticed only on kkt's next server-side refresh; say so; give kkt admin a direct disable. Opaque reference tokens simpler than JWTs.
8. Smaller: a bridge on TCP would be usable by any local process; helper caches access token 0600; Claude.ai holding the human's refresh token for /mcp-user is the one hosted place with human power.

Simpler design: kkt runs a standard OAuth 2.1 AS in login mode; auth code + PKCE (consent page = avatar and swarm choice) for browser-capable clients incl. hosted; device code for headless workers via `kkt connect --swarm X`; orchestrator avatars:create scope; opaque minutes-long access tokens, rotated refresh tokens with reuse detection, per-request state checks, cascading revocation; `kkt token` as headers helper; key mode untouched; DPoP and ID-JAG later, additively.

Verdict: direction right, over-built in three places (token exchange, DPoP proxy, custom approval protocol); replace with standard grants.
