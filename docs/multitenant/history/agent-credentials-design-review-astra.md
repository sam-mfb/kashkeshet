# Avatar credential design review

For Sam and coordinator tawny-ocelot, 2026-11-01. Verdict: keep the separation of human authority from avatar authority, but revise before implementation. The proposed guarantees exceed the design. A kkt issuer is defensible; calling it a small standards-based component understates its responsibility.

Reviewed the proposal and local code at `b206f2489484407336c9d7ad07ff38587766105d`. I did not fetch/switch, to respect the restriction on writes. Provider/client statements below describe documentation, not tested interoperability. Findings are ranked by importance.

## 1. Critical: the hard requirement contradicts the supported execution model

The proposal forbids sibling takeover, then explicitly permits it under the usual shared OS account. A bridge is not an isolation boundary. “Nothing isolates processes under one OS account” is too absolute—sandbox and mandatory access controls can—but ordinary same-account execution supplies no sufficient boundary. Containers help only with appropriate mounts, privileges and process isolation.

Worse, hosted AI clients given `/mcp-user` or `/mcp-admin` authority can exercise human powers through tools. Keeping tokens away from the model does not prevent prompt-injected calls. A human browser session on the worker machine also remains a human credential, even with provider tokens stored server-side.

**Change:** require an isolated execution context for the sibling-isolation guarantee; otherwise promise only no escalation through that avatar's credential. Keep privileged approval in a trusted browser/device outside agent control. Human MCP tools may propose changes, but must not finalize them without independently enforced approval. **Confidence: high.** Evidence: [DPoP §11](https://www.rfc-editor.org/rfc/rfc9449.html#section-11), [MCP security guidance](https://modelcontextprotocol.io/specification/2025-11-25/basic/security_best_practices).

## 2. Critical: approval authenticates a human, not their intended delegation

An agent can substitute its key, request a misleadingly named swarm, send a phishing approval link, or induce a human-authorized client to approve it. Signing the subsequent requests does not repair misapproval.

**Change:** store an immutable pending request binding issuer, tenant, human, avatar, swarm, permission ceiling, key thumbprint, deadline and nonce. Prove key possession before approval; consume approval once atomically. Display server-resolved identities and the exact authority on a trusted origin; never trust agent-supplied labels/HTML. Protect browser actions against CSRF, framing and login/session mix-up. Require deliberate confirmation, not approval by link visitation. Strong authentication still cannot prove that the human understood the request. **Confidence: high.** Evidence: [device-flow phishing considerations](https://www.rfc-editor.org/rfc/rfc8628.html#section-5), [OAuth BCP](https://www.rfc-editor.org/rfc/rfc9700.html).

## 3. High: provider disablement and immediate revocation are overpromised

Successful refresh is not a universal check of the upstream human's enabled status, especially through a broker. Ordinary OIDC sign-in does not promise disablement notification or revocation propagation. Refresh failure can also mean outage, throttling or interaction required. Stopping renewal leaves existing access tokens usable unless local grant state changes. Checking once per request does not stop an already-open wait or stream.

**Change:** define a local grant deadline and maximum identity-status staleness; specify per-provider disablement detection and its measured delay. Consume provider events where available, reconcile missed events, and distinguish permanent denial from transient failure with a bounded fail-closed deadline. Recheck authorization before delayed delivery/commit and cancel revoked waits. Define logout semantics explicitly. **Confidence: high on missing guarantees; provider-specific disablement latency unverified.** Evidence: [Entra refresh lifecycle](https://learn.microsoft.com/en-us/entra/identity-platform/refresh-tokens), [WorkOS sessions](https://workos.com/docs/authkit/sessions), [events](https://workos.com/docs/events).

## 4. High: “RFC 8693 exchange” is underspecified and unnecessary

A server-side session and a raw public key are not automatically RFC 8693 subject/actor tokens. The RFC requires typed security tokens and validation; a key proves possession, not authority to act for a human. `act` is an object identifying the actor, e.g. `{"sub":"avatar-id"}`, not simply a string. Authorizing from human `sub` risks accidentally importing human privileges.

“Subject and actor come from the same request” does not prevent splicing: an attacker can submit unrelated valid credentials together. The cited 2026 report is a report, not an established standards correction; signed JWT contents cannot simply be recombined without invalidating the signature. Nested prior actors are explicitly informational for access control.

**Change:** authorize from the stored approved grant and current avatar, never human privileges or caller-supplied chains. Use token exchange only when there is an actual external token-exchange integration; define its token types and delegation checks then. **Confidence: high; report's claimed general flaw unverified.** Evidence: [RFC 8693 §§2,4.1](https://www.rfc-editor.org/rfc/rfc8693.html), [author's report](https://www.mail-archive.com/oauth%40ietf.org/msg25680/chain-splicing-writeup.pdf).

## 5. High: migration can bypass the new boundary

Current code has two layers: shared HTTP admission (`src/app.rs:154`), then avatar tokens supplied in tool arguments (`src/mcp.rs:71`). `register` creates credentials without avatar authentication; `reclaim` accepts a reusable recovery code (`src/rules/agents.rs:146`). Adding DPoP at HTTP admission while retaining these paths could permit avatar substitution or bearer recovery around it.

**Change:** in login mode derive identity solely from verified transport credentials; remove/reject identity overrides, legacy registration and recovery. Scope every database lookup, channel operation and subscription by tenant/swarm, including tokenless discovery. Keep legacy key mode explicitly separate, with no authentication fallback between modes. **Confidence: high.** Evidence: the cited local files, plus [MCP transport authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

## 6. High: the bridge needs a security contract

DPoP does not sign the MCP body or establish semantic user intent. Its resource proof also requires `ath` (access-token hash), proof time/type/algorithm validation and `Authorization: DPoP`; the proposal's checklist omits these. Replay caches must work across replicas. A bridge accepting arbitrary URLs, avatar selectors or caller headers becomes a signing deputy.

**Change:** pin one approved avatar and server destination per bridge; allow only its MCP transport, with no arbitrary signing/proxy operation or redirect credential forwarding. Use complete DPoP validation, atomic shared replay detection and trusted proxy URI reconstruction. Serialize refresh and design crash/retry handling so rotation races do not routinely revoke legitimate agents. Pin immutable grant ceilings on refresh. **Confidence: high.** Evidence: [RFC 9449 §§4,7,11](https://www.rfc-editor.org/rfc/rfc9449.html), [RFC 9700 §4.14](https://www.rfc-editor.org/rfc/rfc9700.html#section-4.14). Rotation plus key binding is valid hardening; the BCP does not require both.

## 7. Medium: external issuance deserves a real comparison

The Entra claims are substantially correct: refresh tokens span permitted resources, and v2 `aud` is the API application's client ID. But this disproves narrowing a broad grant at redemption, not an external AS issuing a genuinely narrow original grant. Separate clients, resource applications and grant policies can work if the provider enforces them.

**Change:** permit an external AS profile that binds avatar, tenant, swarm, key and immutable permissions, with kkt still checking live membership. Keep minting credentials server-side. Prefer it for deployments already operating a capable AS; do not require bespoke integrations everywhere. **Confidence: high on standards; provider fit requires a prototype.** Evidence: [Entra audience](https://learn.microsoft.com/en-us/entra/identity-platform/access-token-claims-reference), [resource narrowing](https://www.rfc-editor.org/rfc/rfc8707.html#section-2.2).

Vendor claims are broadly supported, with qualifications: [Entra blueprints](https://learn.microsoft.com/en-us/entra/agent-id/identity-platform/create-blueprint) can use federation, not necessarily a stored privileged secret; [Okta exchange](https://developer.okta.com/docs/guides/ai-agent-token-exchange/-/main/) has client/token restrictions; [Auth0 CTE](https://auth0.com/blog/developers-guide-flexible-token-exchange-auth0/) is early access and [DPoP](https://support.auth0.com/center/s/article/availability-of-demonstrating-proof-of-possession-dpop-in-auth0) Enterprise-only. [WorkOS Agent Auth](https://workos.com/docs/authkit/agent-blueprints) targets first-party operated agents; [registration](https://workos.com/docs/reference/agents/registration) alone is not authorization. No checked source establishes portable kkt avatar grants.

## 8. Medium: service-issued credentials are accepted practice, but this is still an issuer

[Kubernetes](https://kubernetes.io/docs/concepts/security/service-accounts/) and [GitHub installation tokens](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app) support the general pattern, not the claimed human-session/PoP semantics: Kubernetes object binding is not key possession, and these examples use bearer credentials. MCP permits a co-located AS; its passthrough prohibition does not require one. JWT libraries do not implement consent, grant policy, refresh families, revocation or issuer operations.

**Change:** use a maintained AS implementation where practical; budget security review, signing-key rotation, audit, encrypted upstream credentials and recovery procedures regardless. Opaque DPoP-bound access tokens are simpler when every request already reads central state; JWTs are optional ([RFC 9068](https://www.rfc-editor.org/rfc/rfc9068.html)). **Confidence: high.**

## 9. Medium: client limitations need narrower wording

[Claude Code helpers](https://code.claude.com/docs/en/mcp#use-dynamic-headers-for-custom-authentication) run at connection time; [Codex](https://developers.openai.com/codex/mcp) caches helper headers. Neither interface supplies a fresh request proof. Native DPoP across all four clients remains **unverified**, not proven absent.

[Claude hosted OAuth](https://support.anthropic.com/en/articles/11503834-building-custom-integrations-via-remote-mcp-servers) and [ChatGPT OAuth](https://developers.openai.com/plugins/build/auth) can receive an avatar-only grant after human consent. They need not receive human powers. This supports an avatar per connector grant, not proven isolation per conversation/sub-agent. Bearer support fails Sam's additional PoP requirement; a remote bridge merely moves that bearer boundary.

**Change:** either allow narrow, expiring, revocable hosted bearer grants explicitly, or leave hosted avatars unsupported pending verified sender constraint. Do not compensate with human/admin access. **Confidence: high on OAuth capability; per-run identity and DPoP unverified.**

## Recommended design and remaining choices

Use one explicit avatar-grant record: owner `(issuer, subject)`, tenant, swarm, avatar, key, permissions, deadline and revocation state. Trusted browser approval creates it. The bridge redeems a one-use key-bound authorization and receives only that grant's credentials. No general token-exchange endpoint is needed initially. Binding this bootstrap to DPoP needs an explicit profile and review, not a claim that RFC 8693 supplies it.

Keep DPoP as the portable CLI adapter. [mTLS](https://www.rfc-editor.org/rfc/rfc8705.html) is reasonable with existing certificate infrastructure but introduces TLS termination/client-support work. [SPIFFE/SPIRE](https://spiffe.io/docs/latest/spire-about/spire-concepts/) suits already-managed workloads; it authenticates workloads, not human consent. [OS keychains](https://developer.apple.com/security/) improve custody, not authorization of every signing caller. Neither helpers nor key storage replaces a request-signing transport.

Approve batches of precommitted worker keys for the strict requirement. Dynamic N-within-T allocation explicitly delegates limited minting: use a separate nondelegable allocation grant, atomic quota, fixed swarm/ceiling/deadline and cascading revocation. State that exception honestly. Preserve existing key mode without mandatory bridge migration. Ship `kkt connect` in the existing binary, with per-client stdio configuration and private per-avatar state; avoid prescribing orchestration frameworks.

[Identity chaining](https://datatracker.ietf.org/doc/draft-ietf-oauth-identity-chaining/) and [ID-JAG](https://datatracker.ietf.org/doc/draft-ietf-oauth-identity-assertion-authz-grant/) are working-group drafts, so “not adopted” is too broad; [agent grants](https://datatracker.ietf.org/doc/draft-oauth-ai-agents-on-behalf-of-user/) remain drafts. The [MCP roadmap](https://modelcontextprotocol.io/development/roadmap) confirms unfinished delegation/DPoP work. None supplies the missing confinement guarantee.

Proceed with a prototype only after correcting the promises and approval boundary. Test cross-tenant substitution, phishing/key substitution, recovery bypass, concurrent refresh/replay across replicas, provider outages/disablement, and revocation during waits—not just token copying.
