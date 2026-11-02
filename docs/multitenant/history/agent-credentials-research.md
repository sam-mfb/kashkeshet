Research report (Opus 5.5 agent, 2026-11-01) on giving agents a smaller scope than the human's identity. Summary of findings; full sources were cited by the agent.

Recommendation (Design A): kkt runs its own small authorization server for avatar tokens only. The human signs in to kkt's web page (kkt as confidential OIDC client; IdP tokens stay on the server). To start an agent, the human approves one RFC 8693 token exchange: subject = the human (server-side session), actor = the agent's own public key. Result: avatar token with sub = human, act = avatar, audience = /mcp only, scope = one swarm, minutes-long lifetime, DPoP-bound (RFC 9449) to the agent's key. Refresh tokens key-bound, rotated, capped by the human's IdP session (kkt refreshes server-side; IdP disable cascades). Central revocation at kkt. Agent tokens never accepted as input to minting (no upward exchange). Works with Entra, Okta, WorkOS today (only plain OIDC sign-in needed). Prior art: Kubernetes bound service-account tokens, GitHub App installation tokens; MCP allows the server's own AS and forbids token passthrough.

Design B (IdP agent identities: Entra Agent ID, Okta OBO): per-IdP, needs a client credential on the machine that is human-equivalent; doesn't cover WorkOS Google/Apple. Doesn't fit.

Design C (optional later): Biscuit attenuable tokens so an orchestrator derives narrower worker tokens offline, each key-bound; orchestrator compromise is bounded to its swarm. Biscuit not an IETF standard; prefer over macaroons.

Don't roll our own: OIDC login/session (use a library); token formats and signing (JWT RFC 9068 or Biscuit libs); proof of possession (DPoP exactly: nonces, jti replay, htm/htu, cnf.jkt); refresh handling (RFC 9700: sender-constrained or rotated with reuse detection); RFC 8693 wire format and act claim, validating subject/actor come from the same request (splicing flaw, Feb 2026); never a bearer-only secret as the agent credential (a one-time code only as a short-lived step that binds a key).

Residual risk: nothing isolates agents under one OS account; a same-user process can read another agent's key or attach to it. Keys in an OS keystore/TPM/Secure Enclave reduce file theft; real separation needs separate OS users, containers or sandboxes. kkt can guarantee no agent credential exceeds its avatar and minting needs a human step; it cannot guarantee siblings under one account can't steal each other's avatar. Document it.

Standards status: RFC 8693, RFC 9449 shipped; identity chaining draft in RFC Editor queue; transaction tokens past WG last call (not for long-running agents); agent-specific drafts (draft-mishra-oauth-agent-grants-02 closest) are individual, unadopted. MCP has no sub-agent delegation mechanism; Enterprise-Managed Authorization (ID-JAG) is per user and client. Entra refresh tokens aren't tied to a resource (90-day default), confirming review point 1.

Caveats: Entra DPoP support and WorkOS per-agent features not confirmed hands-on.
