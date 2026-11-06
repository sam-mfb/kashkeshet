# How kkt's authorization server for avatars is built

Status: deferred by Sam, 2026-11-06. This file collects the question, the options and the evidence so the decision can be taken later. Nothing here is decided, and nothing is implemented.

## Summary

In login mode kkt needs an OAuth 2.1 authorization server that issues avatar tokens for `/mcp` and nothing else. How to build it is open. Two options have evidence behind them:

- **Ory Hydra, a separate service, with kkt in front of its public side.** Hydra is built for an app that supplies its own pages, and it is small and Apache-2.0. Its open-source release lacks two things the current MCP spec recommends, CIMD and `iss`, so kkt would front it to add them. It keeps its own store, so kkt would have to keep two stores in step. It also adds a second program to every login-mode install.
- **The Rust library oauth-as, inside kkt.** It covers every protocol feature the design needs. A prototype kept its state and kkt's grants in one SQLite database, with the decisive changes committed together. But the crate is three months old with one maintainer, draws its own randomness, and has smaller gaps listed below.

Keycloak, Zitadel, Authentik, Rauthy and node-oidc-provider were considered and do not fit as well (section 2.3). Keycloak was looked at again on 26.8.2 and ruled out by Sam on 2026-11-06.

A flaw in this server could expose any avatar on the server, other humans' included. It could not give anyone a human's powers. Separate servers per human would contain such a flaw, at a real cost. A cheaper per-request check would catch one class of mistake, approval-page mistakes; it is proposed below as a candidate.

## 1. The question

How kkt's authorization server for avatars is built: a library inside kkt, or a separate service that kkt configures and sits in front of. Its scope is fixed by the design and is not part of this question:

- In login mode, kkt's own authorization server issues **avatar tokens only**, for `/mcp` only (`agent-credentials-design.md`, sections 2 and 3; `roadmap.md`, section 8.1).
- For humans, kkt is a resource server, never an authorization server. Tokens for `/mcp-user` and `/mcp-admin` come from the org's provider, and kkt verifies them. The web page signs in through the provider as an OpenID Connect client (roadmap 8.1, 8.2, 8.7). Making kkt the authorization server for humans is rejected (roadmap section 14).
- Key mode has no authorization server and does not change.

kkt-creds-expert confirmed this reading in #kkt-dev (message 656).

What the server must do (design section 11):

- authorization server metadata;
- PKCE;
- client identification by CIMD or dynamic registration;
- RFC 9207 `iss`;
- the authorization code grant with kkt's consent page;
- the device grant with kkt's approval page;
- opaque access tokens;
- refresh rotation with reuse detection;
- the token endpoint.

kkt supplies the pages and the grant record either way.

## 2. The options

### 2.1 Ory Hydra as a separate service, with kkt in front

Report: `history/avatar-auth-spike.md` (corrected after review).

**What was tried** (Hydra v26.2.0, SQLite, with a script playing kkt's pages through Hydra's admin API):

- anonymous dynamic registration;
- the authorization code grant with PKCE;
- the device grant with kkt's approval page;
- opaque tokens carrying kkt's grant ID, checked by introspection in about 4 ms;
- refresh reuse killing that grant's tokens;
- revoking one grant on demand.

Footprint: a 70 MB image and about 30 to 60 MB of memory. Hydra ships single binaries for Linux and macOS and is Apache-2.0.

**Gaps found:**

- **No CIMD** in the open-source release. Ory says it will ship only in their paid Enterprise build.
- **No RFC 9207 `iss`**, confirmed in the pinned source.
- **Polling interval not enforced.** The device-grant interval is advice only, with no `slow_down`.
- **RFC 8707 unclear.** Ory's documentation describes RFC 8707 support behind a flag, but the flag is not in v26.2.0 or the public source.
- **Admin API has no authentication.**
- **Slow open-source releases.** The last one was March 2026.

kkt would sit in front of Hydra's public side to serve its own metadata, add `iss`, fetch and register CIMD clients, enforce `slow_down` and rate limits, and keep device-request suggestions.

**Not tested:**

- child credentials for `create_avatar`;
- refresh reuse with a live child or open wait;
- the grant's fixed deadline;
- RFC 8707;
- `iss` on errors.

kkt was not running, so nothing showed a Hydra revocation reaching kkt's grant record.

### 2.2 oauth-as inside kkt

Report: `history/oauth-as-spike.md`. The prototype is at `/workspace/kkt/spike-oauth-as`, outside kkt's workspace. It used oauth-as 1.0.1 over one SQLite database that also held a stand-in for kkt's grant record. It passes the library's own storage test harness and 12 flow tests.

**What was tried:**

- **One commit for the decisive changes.** Approval (device or code) is one commit with kkt's grant becoming active. Refresh reuse is one commit that revokes the token family, kkt's grant, its children and their tokens, and writes the rows that end their waits. A forced failure rolled it all back. Limit: the library itself splits a redemption into a take and later writes, each its own call. A crash between them loses reuse detection for that chain.
- **Protocol features:** PKCE (S256 only), dynamic registration, and `iss` on success, denial and error redirects.
- **RFC 8707:** a wrong or widened resource was refused at the authorization endpoint, the token endpoint and refresh.
- **Device grant:** it works with kkt's page and answers `slow_down` to fast polling.
- **Deadline:** a per-grant absolute deadline held.
- **CIMD:** validation works. kkt fetches the document.
- **Child credentials:** issued through an ordinary device grant that kkt approves on the parent's permission.

**No second program.** The library adds 26 crates with the features used, most of them already in kkt's tree.

**Gaps found:** listed in section 5.

### 2.3 Others considered

- **Keycloak. Ruled out (Sam, 2026-11-06).** Looked at again on 26.8.2, in Docker and from its docs, source and issues: `history/keycloak-recheck.md`. It is the strongest candidate on standard features: `iss` on success and errors, `slow_down`, and refresh reuse ending the whole grant all worked when tried, and CIMD is coming. It is ruled out for kkt because:
  - kkt can't own the consent or device-approval pages without Java plugins on interfaces Keycloak calls internal, which may change in any release. There is no REST way to accept a login or approve a device code.
  - Its users, sessions and consents are a second store beside kkt's grant record, with no per-grant ID and no cascade to children.
  - There is no route for child avatars' credentials: token exchange can't issue another subject's tokens, and device codes are approved only in its own browser flow.
  - CIMD and resource indicators are experimental, with open bugs (among them a server-side request forgery report) whose fixes are on the 27.0 branch only. Preview is targeted at 27.0, due 2027-03-31, with no date for full support.
  - It is the heaviest to ship: a JVM, Postgres or another server database (its file database is not for production), and about 1.25 GB of memory per instance.

  This would change if kkt gave up having its own pages pick the avatar and accepted Keycloak's records as the authority, or if Keycloak shipped CIMD and resource indicators as supported features together with an API for an outside app to supply consent and approve device codes. It would also take kkt being willing to ship only as a Docker deployment with a server database.
- **Zitadel.** Its login pages can be the app's own, but it keeps grants in its own event store. It needs Postgres, has no CIMD or `iss` yet, and has been AGPL since v3.
- **Authentik.** Its pages are its own flows. It needs Postgres and 2 GB, and CIMD is on its roadmap for 2027.2.
- **Rauthy** (raised by Astra, not checked here). A Rust identity provider with the device grant, an embedded database, a small footprint and a published independent security audit. But it is a full identity provider with its own UI. No Hydra-style way for kkt to supply the approval pages was established, nor its CIMD and reuse behaviour.
- **node-oidc-provider** (raised by Astra, not checked here). A Node library with the device grant, dynamic registration, RFC 9207, RFC 8707, opaque tokens and experimental CIMD. kkt would own a Node service built on it, with its storage adapter and page integration, and ship a second runtime. Experimental CIMD may change in minor releases.

### 2.4 Astra's review

Astra reviewed the Hydra spike independently (#kkt-dev, messages 652 to 654).

**Findings:**

- **Hydra's core results are credible.** But "every flow worked" was wrong (since corrected).
- **Two stores break security semantics, not just a page.** When Hydra revokes a grant on refresh reuse, kkt's grant, its children and open waits are not told. Caching a positive introspection hides that revocation. An inactive introspection is not evidence of reuse: expiry reads the same, and the answer names no grant. kkt would need a reliable revocation signal or a specified synchronisation, with retries, reconciliation and tests.
- **Child credentials and the absolute deadline were unproved.** Hydra's refresh lifetime runs from the latest refresh, so it does not enforce a deadline fixed at approval.
- **Fronting Hydra puts kkt inside the authorization server's security boundary.** The CIMD fetcher must defend against server-side request forgery, DNS rebinding, redirects and oversized responses, and must not turn client metadata into arbitrary Hydra configuration. An SSRF bug is worse when kkt can reach Hydra's unauthenticated admin API. `iss` must cover errors and every response mode, without rewriting the redirects in between.
- **Operations grow beyond a second process.** Two databases bring two migration histories, secrets, coordinated backup and restore (a restore must not bring back revoked grants), crash recovery between the stores, and a pinned, tested combination of versions. Key mode must not depend on the service.

**Astra's conditions for keeping Hydra as the leading candidate.** Demonstrate:

- (a) supported child credential issuance;
- (b) exact-once approval and safe retry and recovery;
- (c) absolute grant deadlines;
- (d) refresh reuse revoking kkt's parent grant and its children and ending waits;
- (e) complete resource, `iss` and CIMD behaviour;
- (f) an install, upgrade and restore path that keeps key mode free of the service.

Astra would also budget an independent review of the front end and the synchronisation code.

**Astra's conditions for choosing oauth-as instead.** A bounded spike proves the transitions share kkt's atomic store boundary, its security and maintenance evidence survives review, and its custom code is smaller than Hydra's front end plus synchronisation and packaging. The oauth-as spike met the first in a prototype. The second is open. The third could not be measured, because the Hydra side was never built.

**Astra's conditions for leaving open-source Hydra but keeping a service.** Paid Hydra with CIMD, resource indicators and `iss` removes most of the custom protocol code at an acceptable cost, or a small oidc-provider service meets the approval and revocation contracts more cleanly. Astra would not move to Keycloak just for its longer feature list if kkt-owned approval pages stay a requirement.

## 3. Blast radius of a flaw

From Sam's discussion with sage-cardinal, 2026-11-06.

- **Within the avatar world, it is total.** A flaw in kkt's authorization server, whether in the library, the sidecar, kkt's glue or kkt's approval pages, can expose any avatar on the server, including other humans' avatars. That means reading their closed channels and memory, and posting as them.
- **It cannot give a human's powers.** Those are `/mcp-user`, `/mcp-admin` and the web page. Their tokens come from the org's provider, and avatar tokens are valid for `/mcp` only.
- **Exception: a takeover of the whole kkt server.** That also exposes the provider refresh tokens kkt keeps on the server for the hourly recheck of each human's sign-in (design section 6).
- **With Hydra, an SSRF bug in kkt becomes a way to take over avatars.** Hydra's admin API has no authentication and accepts logins, consents and new clients from whoever reaches it. An SSRF bug anywhere in kkt, for example in the CIMD fetcher, could reach it. A private network or a Unix socket narrows that path but does not excuse an unsafe fetcher.

## 4. Scoping the authorization server per human

Ways to make a flaw affect one human's avatars rather than all of them:

- **(a) A per-human issuer in one process.** One issuer URL and key per human, in one kkt. This adds little, because the approval code and the database are shared, and those are where the flaws would be.
- **(b) Separate servers per human, each with its own database.** This really isolates humans from each other, but it is costly to run. It also needs a separate `/mcp` address per human. MCP discovery names the authorization server through the resource's metadata, and when several are listed the client chooses among them, so kkt could not steer each human to their own.
- **(c) An independent per-request check that the grant's approver owns the avatar.** Cheaper. It catches approval-page mistakes, such as approver A ending up with a grant for avatar B. It does not catch token-handling mistakes, where a real token for B's real grant reaches A. Against those, the only defences are correct code, true isolation as in (b), and detection: showing each avatar's clients and when each was last used, so a human can see a client they do not recognise.

**Proposal (not decided).** Add to the per-request checks in `agent-credentials-design.md` section 5 a candidate check: the avatar's owner is the grant's owner (the human who approved it). It is read from the avatar record independently of the grant and checked on every request beside the others. It is proposed here only; the design file is unchanged until Sam decides.

## 5. Still open before deciding

### oauth-as

- **The crate's maturity.**
  - It is three months old, with 13 releases, one maintainer and one other contributor.
  - It has thorough tests and CI, and forbids unsafe code.
  - The OpenID Foundation listing is a self-certification of 1.0.0 for a FAPI 2.0 profile with DPoP and signed client assertions. That profile does not cover the device grant, dynamic registration, CIMD, opaque tokens with public clients, or MCP.
  - No independent audit was found.
  - The crate's own test suite was not run in the spike.
  - It would need an independent review of the library and of kkt's store.
- **Randomness.** The library draws tokens and codes from `getrandom` itself. That breaks kkt's rule that randomness comes only from the injected `Entropy`. A request to let the host supply token strings was closed as not planned. Using it needs an exception to the rule, or a change upstream.
- **Raw tokens in used codes.** A used authorization code records the tokens it produced, stored as given, until the code expires. The library's harness requires it. Device codes were also stored as given in the prototype.
- **The reuse-revocation event.** If kkt's store fails while revoking after reuse, the client is still told "the grant has been revoked". The library keeps the evidence and reports `containment_failed` through its event hook. kkt would have to handle that event, for example by retrying.
- **Loopback redirects.** A loopback redirect's port may vary only if the registered address carries a port, and `localhost` is not treated as loopback. Claude Code's, Codex's and other clients' actual redirect URIs need checking against this.
- **Not built:**
  - the split across kkt's crates (SQL in `kkt-store`, the library's types in `kkt-avatar-auth`, grant decisions in `kkt-rules`);
  - the CIMD fetcher;
  - rate limiting, sweeping and handling the event hook;
  - creating a child grant in one commit, or recovering when a crash leaves it half done.

### Hydra

Astra's six conditions (section 2.4), none yet shown:

- (a) supported child credential issuance;
- (b) exact-once approval and safe retry and recovery across the two stores;
- (c) absolute grant deadlines;
- (d) refresh reuse revoking kkt's parent grant and its children and ending waits;
- (e) complete resource, `iss` and CIMD behaviour, including errors;
- (f) an install, upgrade and restore path for a second program and database that keeps key mode free of it.

Also:

- the cost and terms of the paid build, if CIMD is wanted from Ory rather than built by kkt;
- whether the RFC 8707 flag ships in an open-source release;
- an independent review of kkt's front end and synchronisation code.

### Either option

- Each MCP client's native login against the chosen server (roadmap section 13).
- A security review, and signing-key and secret rotation, for an issuer kkt operates (design section 11).
- Whether to adopt the approver-owns-avatar check (section 4).

## Sources

- `agent-credentials-design.md`, sections 2, 3, 5, 6, 7 and 11; `roadmap.md`, sections 8.1, 8.2, 8.7, 13 and 14.
- `history/avatar-auth-spike.md` (Hydra, Keycloak, Zitadel, Authentik), `history/oauth-as-spike.md` (oauth-as) and `history/keycloak-recheck.md` (Keycloak 26.8.2), each with its own sources.
- #kkt-dev: Astra's review, messages 652 to 654; kkt-creds-expert on the server's scope, message 656; sage-cardinal's brief, message 660.
- MCP authorization, 2026-07-28 (discovery, client choice of authorization server, CIMD, `iss`): https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization
