Reviewed against `origin/next` at `b206f24` (v0.8.0), after fetching, on 2026-11-01. P0 blocks the promised security boundary; P1 needs resolution before the affected step ships; P2 is a contract, compatibility or scope correction. External checks below are documentation checks, not live provider tests.

1. **P0 — A refresh token can bypass the three-audience boundary.** Sections 2, 8.2–8.3, 8.7; step B.

   `kkt login` leaves refresh credentials with workers, not just a `/mcp` access token. Entra explicitly allows a refresh token to obtain access tokens for other resources permitted to the same client/user. If that client has a prior or admin-granted `/mcp-user` permission, a worker can obtain that token without reading the separate user-token file, then mint codes or reclaim siblings. Checking the audience at kkt does not prevent this exchange. Confidence: high for Entra's behavior; applicability depends on the configured grants. [Microsoft refresh-token documentation](https://learn.microsoft.com/en-us/entra/identity-platform/refresh-tokens).

   Require authorization-server policy that worker refresh grants cannot obtain either privileged resource, including after a human has used `login --user`. Use distinct client registrations and restricted grants where supported; separate files alone are insufficient. Test the actual refresh exchange with a changed resource/scope, not only replay of an existing access token. If a provider cannot enforce this, do not claim the hard boundary for that configuration. This keeps the outside authorization server decision.

2. **P0 — Several Entra identifier URIs are not three v2 audiences.** Sections 8.2, 8.3, 8.5; step B.

   Entra's v2 access-token reference says `aud` is the API's client ID. Three identifier URIs on one app therefore do not provide three distinguishable audiences. Accepting that app ID at all endpoints collapses the boundary; insisting on three URL audiences rejects its tokens. Section 8.2 also configures the root URL while 8.3 requests endpoint URLs. Confidence: high from the token reference; the newer MCP setup guide's looser audience wording makes a captured-token check especially necessary. [Access-token reference](https://learn.microsoft.com/en-us/entra/identity-platform/access-token-claims-reference), [MCP setup guide](https://learn.microsoft.com/en-us/entra/agent-id/secure-mcp-server-with-entra-id).

   Separate the advertised resource URI from the expected token audience in configuration. For Entra, use separate API registrations with distinct audiences, or explicitly design and validate an equivalent scope boundary. Add a real-provider test proving all six cross-endpoint presentations fail. Do not infer audience isolation from successful resource registration.

3. **P1 — Email bootstrap is neither portable nor explicitly one-shot.** Sections 8.4–8.7, 11.

   Configuring an email claim does not establish that it is verified. Entra warns that its email claim is not guaranteed correct and must not authorize access. WorkOS's documented Connect claims do not promise email and verification claims by default. The first-admin path can therefore fail closed indefinitely or become unsafe if implementations treat any signed email as verified. Also, leaving the bootstrap email in config must not grant the role to a later subject given that address, or regrant a deliberately removed role after restart. Confidence: high for Entra; WorkOS claim provisioning needs an actual fixture. [Entra ID-token reference](https://learn.microsoft.com/en-us/entra/identity-platform/id-token-claims-reference), [WorkOS Connect claims](https://workos.com/docs/authkit/connect/token-claims).

   Prefer an issuer plus immutable subject for administrator bootstrap, especially on Entra. If email bootstrap remains, specify trusted verification evidence, atomically consume the bootstrap, and bind the resulting role to the subject permanently. Make host-driven recovery an explicit new bootstrap operation. Test address reuse, concurrent first logins, restart, and migration of the old user's ownership.

4. **P1 — First-sign-in admission leaves later org claims ambiguous.** Sections 4, 8.4, 8.7; step E.

   WorkOS tokens carry the organization selected for that authorization. A subject previously admitted to org A may later present a valid token selected for org B. Looking up only `(issuer, subject)` can give a B-authorized connector access to A. Likewise, automatic claim admission must not recreate or reactivate a locally removed user. The document says the user row is checked, but does not define these outcomes. [WorkOS organization access](https://workos.com/docs/authkit/connect/oauth#organization-access).

   Specify per-request matching of organization-bearing grants to the user's stored org, without silently moving the user. Define invitation-only grants separately. Keep a disabled subject record that automatic admission cannot override. Reject conflicting claim bindings and claim/invitation matches to different orgs rather than choosing by iteration order. Test a changed or absent `org_id`, a removed user signing in again, and concurrent invitations.

5. **P1 — Org admins must not control the admission boundary of other orgs or grant server power.** Sections 5, 8.4, 10; step E.

   Section 8.4 assigns claim binding to the server admin, but sections 5 and 10 give org admins login settings, explicitly including claim binding in section 10. An org admin able to bind another organization's claim can capture its new users. The generic permission to manage “roles” similarly needs to exclude server roles.

   Reserve issuer/claim-to-org bindings and server-role grants to the server capability. Org admins may manage only their own org roles and invitations. Enforce uniqueness of bindings and same-org membership for group members, swarm owners, shares and avatar/channel relationships. Test these writes, not just whether one capability can read the next scope. A private Rust capability does not itself enforce the relationships between supplied record IDs.

6. **P1 — Login needs a browser transaction contract and access-token profile.** Sections 8.1–8.2, 8.5–8.7; step B.

   “Verifies the id_token” and returning an identity triple leave critical behavior unspecified. Today's `web/sign_in.rs` accepts a local key; its form-token protection does not implement an OIDC callback. The verifier contract also lacks an explicit target resource and expiry, which later authorization and long waits need.

   Specify a short-lived, single-use sign-in transaction bound to the initiating browser, state/nonce and PKCE, a fixed callback and safe return target, ID-token audience equal to the page client ID, and separation from API access-token validation. Account for today's `SameSite=Strict` cookie on the cross-site return. The access-token result should include verified subject, target resource, granted scopes and expiry, and distinguish human grants from application-only grants. Pin allowed algorithms; reject future `nbf`, missing required claims and wrong token kinds. Bound unknown-key refreshes. Add negative callback/replay/token-profile tests. [OIDC code flow and validation](https://openid.net/specs/openid-connect-core-1_0.html#CodeFlowAuth), [MCP authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

7. **P1 — Revocation must cover idle streams, credentials and web sessions, starting in B.** Sections 4, 8.7; steps B and D.

   Today's `rules/messages.rs:241` authenticates before waiting and later reads by avatar handle; it ignores non-message notices. `web/real.rs` forwards the global event subscription, and `web/mod.rs:759` opens it without retaining the viewer's session. Per-event visibility checks alone do not end an idle subscription when its bearer expires, its session signs out, or an org is suspended. The 12-hour page cookie also outlives an expired provider access token, contradicting the unqualified provider-removal claim in 8.7.

   Carry the caller and credential deadline into waits/feeds, wake or cancel on authorization changes, and recheck before returning data. Define the web session's separate revocation window. Include token replacement by reclaim, logout, role removal, user deactivation and org suspension, with no new messages arriving. Run these tests in B; group removal in D is too late to establish login-mode revocation.

8. **P1 — Scoping the event subscriber is not enough for compound events and page reads.** Sections 4–5; step A.

   `events.rs` includes `AgentHidden.channels`, membership counts and a global `Missed(n)`, not only message events. `rules/human.rs::agent_view` bundles full memory into an otherwise public avatar view; `admin_agent` reuses it. A user who may see a colleague's avatar in a shared swarm must not thereby receive its memory. `overview` also bundles server-run settings with ordinary channel data.

   Make the inventory promised in section 5 a concrete checklist: nested event fields, lag counts, member previews, waiting-channel lists, memory, aggregate counts, server settings and hidden/deactivated records. Separate public avatar details from owner/admin memory reads and server-only run details. Test complete serialized results with foreign markers, pagination and slow subscribers. Keep event authorization in rules, preserving `events.rs`'s boundary. Do not accidentally add global lag counts as a third metadata exception.

9. **P1 — B is not independently shippable with the current placement of permissions and pages.** Section 12.

   B admits multiple users, but C introduces user pages, D makes the access rule/transition table live, and E introduces assigned org admins and protects server-announcement tools in login mode. B already needs ordinary-user pages, working local deactivation, an org admin and a server-role check. Otherwise the first admin cannot perform required administration, or non-admin logins inherit today's operator powers. `app.rs::admit` currently discards the principal; `AdminServer` is cloned globally and `RulesSource` is shared globally.

   Put request-scoped identity propagation, all existing-tool role checks, bootstrap org-admin assignment, minimal user pages and account/session revocation in B. D should add shared-swarm transitions only; E should add multiple-org admission only. Delay `login --user` until C's endpoint exists. Give C explicit owner-only read/reclaim/deactivate tests. Test B with a server admin, org admin and ordinary user through actual HTTP surfaces, not only seeded rule calls.

10. **P1 — Mint-code lifecycle and failed redemption need explicit rules.** Sections 3–4, 8.3, 8.7, 9; steps B and D.

   Section 9 says the code fixes the owner; the main model makes the redeemer the owner. Codes store no creator, so the described record cannot support a user's code inventory, per-user outstanding-code limit or revoking that user's codes. Access alone remains true for humans reading a deactivated swarm, so it is not enough to permit redemption. A code may also outlive its creator's access.

   State that the redeemer owns the avatar; record the code creator separately. Define explicit revocation and what happens on creator removal/access loss. Check active user, org and swarm plus current access inside the same transaction that decrements uses and creates the avatar. A taken handle, invalid lane or failed insert must spend no use. Test competing redemption of the last use and redemption racing deactivation. Give “short random” a concrete entropy floor and bound failed guesses as well as successful registrations; use the existing secret generation mechanism where possible.

11. **P1 — `/mcp-admin` channel creation has no way to choose a swarm.** Sections 4–6, 9; steps B and D.

   `mcp_admin.rs::CreateChannelParams` has name, visibility, purpose and members, but no swarm. An org capability identifies an org, not one of its swarms. Open channels can have no initial members, so inferring the swarm from members cannot cover today's tool. Picking the admin's personal swarm silently would make the existing administrative surface misleading.

   Add an optional swarm parameter, with the existing one-swarm default in key mode and an explicit required/ambiguous result in login mode. Validate it inside the org capability and enforce same-swarm initial members. Preserve the distinction between naming a target inside an authorized org and choosing an unauthorized org scope. Pin the revised admin schema and behavior in contract tests.

12. **P2 — Moving sends to per-user limits silently reduces existing capacity.** Sections 8.8, 11; step A.

   Sends are already per avatar, not per server: `rules/rate.rs::SendLimiter` keys by agent and `config.rs` defaults to 30 per minute. Migrating all old avatars to one user and applying that number per user changes N avatars from N separate buckets to one. Configurable limit fields also must not become constants as 8.8 suggests.

   Keep existing per-avatar send limits in key mode. Add a separately named user budget for login mode if needed, or explicitly obtain approval for this additional compatibility break. Test two migrated avatars using their independent existing budgets and loading old limit settings.

13. **P2 — The deactivation change breaks more contracts than A's stated exception.** Sections 4, 9, 11–12.

   Today's `hide_agent` refuses channel owners (`rules/administration.rs:152`); stale cleanup deliberately keeps owners too. “Never refused” deactivation changes those semantics, beyond removing already revoked agents from lists. It must still refuse unauthorized callers. The migration test cannot both keep every old read identical and remove revoked memberships/ownership. Notices are also required for channels an avatar owns but has left; today's `remove_avatar_in` handles these explicitly, while section 4 mentions only channels it is in.

   List the exact approved behavior changes, preserve old tool names as aliases, and decide whether stale cleanup retains its conservative owner rule. Interpret “never refused” as no ownership precondition after authorization. Specify owner-left notices and whether migration emits notices. Compare old and new reads with explicit exceptions rather than a blanket equality claim; add FTS, foreign-key and historical-author checks when rebuilding handle-based references.

14. **P2 — Swarm ownership/removal has unresolved edge cases.** Sections 3–4; step D.

   Removal is refused while a user owns a shared swarm, yet deactivating that swarm is offered as a way to unblock removal without changing ownership. Reassigning a swarm can also collide with the new owner's existing swarm name. Personal-swarm ownership reassignment is not expressly prohibited, despite the one-person/one-personal-swarm invariant.

   Say whether only active shared swarms block removal and who owns/accesses archived swarms afterward. Refuse personal-swarm reassignment. Define collision handling for reassignment, and require an active same-org new owner. Test loss of the last grant and retention of another grant separately; permanent avatar deactivation should follow the final union of grants, as the design intends.

15. **P2 — The human channel-change rule needs one authorizing avatar, and ownership history needs a real actor.** Sections 5 and 7.

   A human may change a channel “exactly as that avatar could,” but could have one avatar that owns the channel and has left it, and another that is a member. Combining those facts would grant a power neither avatar has; today's `access::decide` requires membership and ownership together. Separately, “an org admin” is adequate public wording but cannot identify which of several admins changed a role, claim binding or swarm owner. Current records often use null for the single human operator.

   Require one active owned avatar to satisfy the complete channel action, chosen internally or explicitly. Persist the authenticated user ID and relevant client attribution for administrative changes, while keeping public notices anonymous as planned. Do not treat the self-reported MCP client name as authenticated attribution. This needs a small actor record, not a new general audit framework.

16. **P2 — A generic OIDC implementation needs a narrower interoperability promise.** Sections 8.5–8.6, 13.

   OIDC does not guarantee that a page ID token and three resources' access tokens expose the same subject: pairwise subjects are valid. Entra's `oid` choice addresses one provider, but “same code path” does not establish the mapping for others. OAuth also does not guarantee a refresh token on every authorization, though 8.2 describes one as automatic. Confidence: high. [OIDC subject rules](https://openid.net/specs/openid-connect-core-1_0.html#SubjectIDTypes), [MCP refresh guidance](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization#refresh-tokens).

   Require a documented stable identity claim across the supported page/client/resource combinations; never repair a mismatch by automatically linking on email. Test the same human through all surfaces. Handle absent refresh tokens with reauthentication. Keep the mock small and deterministic; a complete custom provider with CIMD fetching cannot prove compatibility with real providers. Gate B on captured, sanitized claim fixtures and an actual supported-provider login/refresh smoke test, including audience isolation from items 1–2.

17. **P2 — Correct and narrow the external compatibility checklist.** Sections 8.2, 8.5, 8.8, 13.

   Okta's custom authorization server and production API Access Management requirement are documented, not still an open factual question. Its ordinary org issuer cannot mint access tokens for kkt. WorkOS documents CIMD and resource-specific audiences, but its default Connect claim table does not establish verified email for invitation/bootstrap; configure and test that separately. Confidence: high for documented features, unverified for this project's end-to-end combinations. [Okta authorization servers](https://developer.okta.com/docs/concepts/auth-servers/), [WorkOS MCP configuration](https://workos.com/docs/authkit/mcp).

   ChatGPT's callback is not always variable: its current documentation gives a stable callback when the authorization server supports the required issuer-identification behavior, with the callback-ID URL as a fallback. Use the exact connection-management value rather than a universal template. Confidence: high. [OpenAI authentication](https://developers.openai.com/plugins/build/auth). Claude's remote connector model supports the claimed hosted route, but does not establish that every desktop/local integration has the same callback or reachability; label the table as remote connectors. [Claude connector documentation](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp).

   EMA was announced stable on June 18, 2026, not July 28. “Nothing from kkt” should mean no additional token-exchange endpoint in kkt; it still requires provider/client configuration and the same audience, grant and identity checks. [MCP announcement](https://blog.modelcontextprotocol.io/posts/enterprise-managed-auth/).

18. **P2 — Explicitly preserve pre-avatar discovery and recovery contracts.** Sections 6–9; steps B and E.

   “Every tool” being bound to an avatar excludes today's token-free `list_lanes`, and `register`/`reclaim` inherently run before possession of an avatar token. `mcp.rs::initialize` and `discover` also return announcements before registration. `reclaim` currently returns the server announcement, lane and memory summaries, but is missing from the new announcement output table. These are observable agent contracts, not just implementation details.

   List the exceptions: discovery and `list_lanes` use the authenticated user's org; registration uses the mint code; recovery requires both the owning user and the recovery code in login mode. Recheck ownership before rotating the token or returning lane/memory data, including for a valid code belonging to another user. State which announcement fields `reclaim` gains. Keep key-mode behavior and update instructions/tool descriptions and both contract snapshots in each affected step.
