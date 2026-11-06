# Spike: oauth-as inside kkt

Status: research for a decision Sam has not made, 2026-11-06, by kkt-authspike-2. It follows Astra's review of the Hydra spike (#kkt-dev, messages 652 to 654), which named the Rust library oauth-as as a possible alternative to a separate Hydra service and set conditions under which it would win. Nothing in kkt was changed. The prototype is a throwaway Cargo project at `/workspace/kkt/spike-oauth-as`, outside kkt's workspace, using oauth-as **1.0.1** (pinned `=1.0.1`, features `http` and `cimd`) and rusqlite 0.40.2.

## Summary

The prototype is a SQLite store that implements oauth-as's storage interface and holds a stand-in for kkt's grant record in the same database. The library's own storage test harness passes against it. On top of it, 12 tests drive the library's real token, device, registration and metadata endpoints in-process.

**Astra's first condition, one atomic store boundary, is met in the prototype, with limits.** The decisive changes each commit once, together with kkt's grant:

- **Approval.** The approval (the device code marked approved, or the authorization code issued) and kkt's grant going from pending to active are one transaction. Without a pending kkt grant, the approval is refused and nothing changes.
- **Refresh reuse.** One transaction removes the reused token's family, revokes kkt's grant and all its children, removes the children's tokens, and writes the rows that would end their waits. A forced failure in kkt's half rolled the whole thing back.
- **Deadlines and revoked grants.** Every token or refresh write checks kkt's grant in the same transaction, so a revoked or expired grant gets no new tokens.

The limit is that the library itself splits a redemption into a "take" and later writes, each its own call. Its documentation says so. A crash between the two calls loses the record that detects reuse for that chain. The library's opt-in retry window makes rotation one call, but it delays theft detection.

**The other two conditions are not settled by this spike:**

- **Review evidence.** The crate shows unusual engineering care: thorough tests, a harness written independently of the library, `forbid(unsafe_code)`, and an OpenID Foundation listing. But it is three months old and has one maintainer. Its certification is a self-certification of version 1.0.0 for a FAPI 2.0 profile that uses DPoP and signed client assertions. That profile does not cover the device grant, dynamic registration, CIMD, kkt's opaque-token and public-client setup, or MCP. No one outside the project has reviewed it that I could find, and I could not run its test suite here.
- **Code size.** The library route measured about 650 lines of store code plus about 220 lines of kkt grant logic, which either route needs. The Hydra route's front end, synchronisation and packaging were never built, so there is no measured number to compare against.

**What the library did, tried:**

- **Device grant.** It works with kkt's own approval page through `approve_device`, and it answers `slow_down` to fast polling.
- **Authorization code.** PKCE is S256 only. `iss` appears on success, on denial and on error redirects, and the metadata advertises it.
- **Resource indicators (RFC 8707).** The wrong resource is refused at the authorization endpoint, at the token endpoint, and when a refresh tries to widen the grant.
- **Refresh.** Reuse revokes the token family. A per-grant absolute deadline works once kkt's store caps each chain at the grant's deadline.
- **Dynamic registration and CIMD.** Both work. CIMD is validation only: kkt fetches the document.
- **Child avatar credentials.** These come from an ordinary device grant that kkt approves on the parent's permission. The library has no API for issuing tokens to a new subject directly.

**What is left, or does not fit:**

- **Randomness.** The library draws its own randomness (`getrandom`), which breaks kkt's rule that randomness comes only from the injected `Entropy`. A request to let the host mint token strings was closed as not planned.
- **Two kinds of secret stored as given.** The tokens recorded inside a used authorization code must be stored exactly as given until the code expires; the library's harness requires it. Device codes are also stored as given in the prototype.
- **Splitting the store.** The prototype keeps the store in one file. kkt would have to split it: SQL in `kkt-store`, the library's types in `kkt-avatar-auth`, grant decisions in `kkt-rules`. Not built.
- **Child creation is several commits.** Spending the parent's allowance and writing the pending child grant is one commit. Approving the device request and redeeming it are separate ones. A crash between them leaves a spent allowance and a dead pending grant.
- **A failed reuse revocation is easy to miss.** The client is still told "the grant has been revoked". The real signal is an audit event (`containment_failed`), which kkt would have to handle. Not built.
- **Loopback redirects.** A port-less registration such as `http://127.0.0.1/callback` matches only port-less requests, and `localhost` is not treated as loopback. Each MCP client's actual redirect URIs need checking against this.
- **Host duties not wired in the prototype.** The library leaves rate limiting, the sweep of expired records, and waking waits from the outbox to the host.

## The nine points

| # | Point | Tested | Read only | Not done |
|---|---|---|---|---|
| 1 | One store, one commit | Approval, both grant types; strict rotation; reuse revoking family, kkt grant and children in one commit, with forced rollback; library storage harness | Retryable rotation (implemented, harness-checked, not in a flow test) | Split across `kkt-store`, `kkt-rules`, `kkt-avatar-auth`; waking real waits |
| 2 | Child credentials | Pre-approved device grant: issue, refresh, count spent, child refused, parent revocation cascades | Library has no direct issuance API; token exchange exists but the design rules it out | Crash recovery between the steps |
| 3 | Absolute deadline | 10-day grant refused after day 10 though rotated daily | Library's server-wide absolute chain lifetime | |
| 4 | Device grant | kkt's page via `approve_device`; `slow_down`; wrong, expired, reused codes; suggestion kept by kkt | Library's rate-limit hook and its own CSRF-protected form | Rate limiter wired |
| 5 | PKCE, DCR, `iss`, RFC 8707 | All, including errors and refresh widening; loopback ports | | Against real MCP clients |
| 6 | CIMD | Validation refusals; validated document registered and used | What the library leaves to the host | Fetcher (no redirects, address checks, size, time, cache) |
| 7 | Fit with kkt's rules | Injected clock; tokens and codes keyed by hash | Randomness, axum, third-party types, dependency tree | Real crate split |
| 8 | Crate health | | Repository, releases, issues, tests counted, licence, unsafe, OIDF listing | Running its test suite (refused by this session's sandbox); independent audit |
| 9 | Code size | oauth-as route measured | | Hydra route: not built, so not measured |

## 1. One store, one commit

**How it is built.** The store implements oauth-as's `Storage` trait over one SQLite connection behind a mutex, so each method is one SQLite transaction and none overlap. It follows the semantics of the library's own PostgreSQL store (`crates/oauth-as-postgres`), including its revocation barriers: rows that stop an in-flight write from restoring something a revocation removed. The OAuth subject of every token is a kkt grant ID. In the same database are a `kkt_grants` table (grant ID, owner, avatar, permissions, a remaining count for creating avatars, deadline, state, revocation details, parent grant, client, run), a table mapping refresh-token families to grants, an outbox of revocation events, and kkt's device-request suggestions. kkt's rules run inside the store's transactions:

- **Approval.** The device approval (`compare_and_swap_device_grant`) and the issuing of an authorization code (`put_authorization_code`) move kkt's grant from pending to active in the same transaction. If there is no pending grant, the approval is refused and nothing is written. Tried: approving with no pending grant was refused (`NotPending`); approving with one made it active; a second approval of the same code was refused and left the second grant pending. Issuing a code made the grant active.
- **Used exactly once.** Taking a device grant, authorization code or refresh token is a single `DELETE ... RETURNING`. Tried: a second redemption of a device code gave `invalid_grant`. The library's harness races these and passed.
- **Each token write checks the grant.** Writing an access token or an active refresh record is refused if kkt's grant is not active or is past its deadline.
- **Refresh rotation (the library's default, strict mode)** is several calls: take the old refresh token, write it back marked spent, write the new access token, write the new refresh token. Each is its own transaction with kkt's check inside. The library documents that a crash between the take and the writes loses the spent marker, which disarms reuse detection for that chain. With the opt-in `refresh_retry_window`, rotation is one call (`rotate_refresh_token`), which the prototype implements as one transaction with kkt's checks. The library warns this delays theft detection, since a thief can retry within the window.
- **Reuse.** The library calls `revoke_token_family`. The prototype does all of this in one transaction:
  - records the family's barrier and deletes its tokens;
  - finds kkt's grant through the family map;
  - revokes that grant and every descendant;
  - records barriers for, and deletes, the descendants' token families;
  - writes one outbox row per revoked grant, for kkt to end their waits after the commit.

  Tried: a parent and a child each held live tokens and an unrelated grant held its own. Presenting the parent's rotated-away refresh token gave `invalid_grant`. Both grants were then revoked ("refresh token reused", "parent revoked"), all their tokens were refused, the unrelated grant kept working, and the outbox held two rows.
- **Rollback.** With a switch that makes kkt's half fail after changing its rows, the same reuse left everything as it was: grant active, tokens present, no barrier. The library still told the client "refresh token reuse detected; the grant has been revoked", although nothing was revoked. By design it put the spent token back and emitted `RefreshTokenReuseDetected` with `containment_failed: true` to the host's event hook. Presenting the token again, with the failure switched off, revoked everything. kkt would have to handle that event, for example by retrying the family revocation. Not built.
- **The library's harness.** `oauth_as::storage_conformance::check_storage` (feature `test-util`, about 60 named checks, some for features not enabled here) reported no violations after two fixes. The first was to refuse an empty client ID. The second was to store the tokens inside a used authorization code exactly as given, since the harness requires them to round-trip (see point 7). The harness uses subjects that are not kkt grants, so it does not exercise kkt's rules; the flow tests do.

**Where it would break in kkt as it is.** kkt's rules say only `kkt-store` writes SQL, permission decisions belong in `kkt-rules`, and third-party types stay in the crate that wraps them. The prototype ignores all three by keeping everything in one store file. A real version would:

- implement the library's `Storage` in `kkt-avatar-auth`, turning its records into rows of our own type with the record as an opaque JSON field;
- give `kkt-store` plain methods for those rows;
- run kkt's grant rules inside `Db::write` closures.

The single transaction is still possible that way, because `Db::write` is one SQLite transaction, but it was not built. The library's async trait over kkt's synchronous database also needs a decision: a mutex as here, or blocking tasks.

## 2. Child credentials

The library has no supported call for "issue tokens to this new subject" outside a grant. Its options:

- **Token exchange (RFC 8693, feature `token-exchange`).** The design rules it out (`agent-credentials-design.md`, section 11).
- **Client credentials.** No refresh token, and no subject.
- **Host-chosen token strings.** Requested in oauth-as issue 25, closed as not planned on 2026-11-02.

What was built uses only standard grants. `create_avatar` takes these steps:

1. In one kkt transaction, spend one of the parent's allowance (the parent must be active, within its deadline, hold `avatars:create` and not be a child itself) and write a pending child grant whose deadline is the parent's.
2. Start an ordinary device grant for the `kkt` command's client.
3. Approve it at once, on the parent's permission, for the child grant. This is where the child grant becomes active.
4. Redeem the device code at the token endpoint and return the tokens once.

Tried:

- The child's token was accepted and its refresh token rotated.
- A second `create_avatar` was refused once the count of one was spent.
- A child trying to create was refused.
- The child's deadline equalled the parent's.
- When the human revoked the parent, the child was revoked and its token refused.

Gaps:

- **Several commits.** The steps after the first are separate commits. A crash in between leaves a spent count and a pending child grant nobody can use.
- **A device grant with no device.** It uses the device grant with no device and no human at that step, approved by kkt on a permission the human set earlier. Whether that is acceptable is for Sam and the reviewers.

## 3. Absolute deadline

- **Server-wide.** The library supports an absolute refresh-chain lifetime for the whole server (`refresh_token_ttl`), counted from when the grant was approved. Rotation does not extend it.
- **Per grant.** kkt's deadline differs per grant: a child's is its parent's. The store caps each refresh record's expiry at the grant's deadline when it writes it, and every token write checks the deadline as well.
- **Tried.** A grant with a 10-day deadline, under a 30-day server-wide limit, was refreshed every day for 9 days. Its stored chain expiry equalled the deadline. On day 10 plus one second, refresh failed with "refresh token chain expired". kkt's per-request check also refuses a token whose grant is past its deadline.

## 4. Device grant

Tried, through the library's HTTP endpoints:

- The device request returned a code valid for 600 seconds and a 5-second interval.
- The first poll got `authorization_pending`. An immediate second poll got `slow_down`, and after waiting the raised interval, `authorization_pending` again.
- **kkt's approval page.** The page would call `approve_device(user_code, grant_id)` itself; the library's own verification form was not used. A wrong code was refused (`UnknownUserCode`). After approval, a poll returned the tokens and a second redemption gave `invalid_grant`. An hour later an unused code gave `expired_token`, and approving it was refused.
- **The requester's suggestion.** The device request carries only the client and scope, so kkt keeps the suggested handle and purpose in its own table under the normalised user code. Its page reads them back to show as the requester's claim. kkt writes this row after the library's call, as a separate commit.
- **Rate limiting.** The library asks a host-installed rate limiter before each user-code lookup and ships a fixed-window one (read only). The prototype did not install it; kkt's limits per user and per minute would.

## 5. PKCE, dynamic registration, `iss`, resource indicators

- **Metadata.** It advertises `authorization_response_iss_parameter_supported: true`, `client_id_metadata_document_supported: true`, `code_challenge_methods_supported: ["S256"]`, the registration and device endpoints, and only the `query` response mode.
- **PKCE.** A request with no PKCE, or with `plain`, was refused by an error redirect.
- **Dynamic registration.** It is off unless configured and a host policy allows it. An open request returned 201 with a registration access token. A request for a grant type outside the policy got `invalid_client_metadata`.
- **`iss`.** It was present on the success redirect, on the denial redirect (`access_denied`), and on every error redirect tried (PKCE errors and `invalid_target`). Errors about the client or the redirect URI are shown directly rather than redirected, as RFC 6749 requires.
- **Resource indicators at the authorization endpoint.** The server allowed `/mcp`, plus `/mcp-user` only so widening could be tested. `/mcp-admin`, a relative URI and one with a fragment were refused with `invalid_target`. `/mcp` was accepted, `/mcp` given twice was accepted, and two allowed resources were accepted.
- **Resource indicators at the token endpoint.** A resource the grant did not include gave `invalid_target`, and that failed attempt used up the code, so the correct retry got `invalid_grant`.
- **Resource indicators at refresh.** No resource worked, asking to widen to `/mcp-user` gave `invalid_target`, and `/mcp-admin` gave `invalid_target`.
- **Loopback redirects (RFC 8252).** A client registered with `http://127.0.0.1:8080/cb2` was accepted at any port on the same path. One registered with `http://127.0.0.1/callback` (no port) was refused at a port. `localhost` was refused. This is the 1.0.1 fix for issue 26. Whether Claude Code's and Codex's redirect URIs fit it needs checking against the clients.

## 6. CIMD

**What the library checks (tried):**

- The URL must be `https`.
- An IP-literal host in special-use ranges is refused (`10.0.0.5` and `127.0.0.1` were refused).
- A query string is refused.
- The document's `client_id` must equal the URL.
- `jwks` key material is refused.
- Shared-secret authentication methods are refused.
- The document's size is capped.
- The grants and scopes a document may claim are capped by the policy. A document with no `scope` became a client allowed no scopes, so kkt would need to set the scope itself.
- By default redirect URIs must share the document's origin. The prototype turned that off for native clients' loopback redirects.

A validated document becomes an ordinary client, which kkt registers before handing the authorization request to the library. That worked. The library follows draft revision -01; the MCP spec cites -00, and the current draft is -02. The module maps its section numbers onto -02 and says the rules did not change.

**What kkt must still do (from the module's own list):**

- Recognise a URL client ID at the authorization endpoint.
- Fetch the document with no redirects, accepting only a 200, with a time limit and a size limit.
- Resolve the name once, check that address against private and special ranges, and connect to that address.
- Verify TLS.
- Cache and refresh per the draft, never caching a failure.
- Decide what happens when a document changes while grants from it are live.

None of this was built. The Hydra route needs the same fetcher.

## 7. Fit with kkt's rules

- **Clock.** Injected (`AuthorizationServer::with_clock`, a `Clock` trait with `now()`). The tests ran entirely on a test clock, including a 10-day deadline and device timing.
- **Randomness.** Not injectable. Tokens, codes and IDs come from `getrandom` inside the library (`server.rs`). This conflicts with kkt's rule that randomness comes only from `Entropy`. Tests still pass, because nothing depends on the values. Using the library needs an exception to that rule.
- **Third-party types.** The `Storage` trait is written in the library's record types. They can stay inside `kkt-avatar-auth` if `kkt-store` stores them as opaque JSON. The library's serialised record format then becomes part of kkt's database. The library promises that persisted records decode unchanged across compatible upgrades; that was read, not tested.
- **axum.** Not needed: the `http` feature uses only the `http` 1.x types, and kkt can mount the service under its own router. The optional `axum` feature uses axum 0.8, the same major version as kkt's 0.8.9.
- **Stored hashed.** Access tokens, refresh tokens and authorization codes are keyed by SHA-256, with the value blanked in the stored record. kkt's per-request check looks the token up by its hash. Exceptions:
  - The tokens recorded inside a used authorization code are stored as given until the code expires (`authorization_code_ttl`), because the library's harness requires them to round-trip and the library uses them to revoke on code replay.
  - Device codes are stored as given in the prototype, for their 10 minutes.
  - Client secrets can be stored as hashes through the library's `SecretHash`, and the registration access token is stored as a hash by the library (read only).
- **Host duties.** `sweep_expired` on a timer, the rate limiter, and handling the event hook. All are seams the host must wire; none were wired here.
- **Dependencies.** With `http` and `cimd`, the library's tree is 26 crates. Eight are not in kkt's `Cargo.lock` at those versions; seven of those are second versions of crates kkt already uses (for example `sha2` 0.10 beside kkt's 0.11, `getrandom` 0.3, `base64` 0.22).
- **Cost per request.** kkt's check from a token to an active grant within its deadline is one indexed read: about 22 µs per call in a debug build with an in-memory database. There is no second service to call.

## 8. Health of the crate

**Releases.** The repository and first release date from 2026-08-08. There have been 13 releases: 0.9.0 to 0.11.2 between August and September, 1.0.0 on 2026-09-24, and 1.0.1 on 2026-11-02 (the loopback-port fix and a security update to an optional dependency). crates.io counts 44,606 downloads of 1.0.0 and a few hundred or fewer for each other version.

**People and issues.** 231 of 236 commits are by Matthew Jackson. One other person contributed a change (PR 10, the refresh retry window); the rest are dependabot. The repository has 10 stars and 4 forks. Users have opened two issues: 26 (loopback ports, fixed in 1.0.1) and 25 (host-minted tokens, closed as not planned). In issue 25 the maintainer mentions their own product using it for MCP flows; I did not verify that.

**Tests and CI.**

- 1,558 test functions in the library crate.
- About 74,000 lines of tests against about 40,000 lines of source.
- A separate conformance harness written by someone who could not see the library's source (README).
- A pinned third-party client run against it.
- CI includes CodeQL, OpenSSF Scorecard, supply-chain checks and the FAPI 2.0 conformance suite.

I did not run the suite: this session's sandbox refused it. The library's storage harness did run, against the prototype's store.

**Licence and safety.** MIT or Apache-2.0. `#![forbid(unsafe_code)]`.

**The OpenID Foundation listing.** The FAPI 2.0 Security Profile page lists "Matthew Jackson | oauth-as 1.0.0 | 24-Sep-2026".

- **What it is.** Per the README, a self-certification to the "FAPI2SP OP private key + DPoP" profile, covering 1.0.0 only. The Foundation publishes the result but does not itself verify the claim.
- **What it does not cover:**
  - the device grant, dynamic registration, CIMD or RFC 8707 as kkt would configure them;
  - opaque tokens with public clients and no DPoP, which is kkt's setup;
  - any host's store;
  - MCP.

**Reviews.** The README describes the project's own adversarial review and mutation testing. I found no published independent audit cited.

## 9. Code size

**oauth-as route, measured in the prototype** (lines of code, not counting blank lines and comments):

| Part | Lines | Which route needs it |
|---|---|---|
| SQLite store for the library | 652 | oauth-as only. Includes about 60 lines of kkt's rules inside its transactions, and the schema |
| kkt grant stand-in | 223 | Either route: grant table, cascade, permission count, per-request check |
| Tests | 628 | The library harness plus 12 flow tests |

Not built:

- The split across kkt's crates, which adds an adapter layer.
- The CIMD fetcher, which either route needs.
- Wiring the rate limiter, the sweep and the event hook.
- The consent and approval pages, which either route needs.
- Migrations.

No second process, image, binary, database or backup.

**Hydra route.** The Hydra spike built no kkt-side code. It was a 210-line Python script driving Hydra's APIs. So there is no measured number for the Hydra route. What it would need, from the Hydra spike and Astra's review:

- **A front end** for metadata, `iss` on success and error, the CIMD fetcher and admin registration, `slow_down` and rate limits, device-request suggestions, and checks on `resource`.
- **Two-store synchronisation:** revocation in both stores, retries and reconciliation, and detecting reuse revocations made by Hydra.
- **Packaging:** a Compose service, binary download and checks for launchd and systemd, generated configuration, Hydra's migrations on upgrade, backup and restore of a second database, and its secrets.

A fair comparison of size needs the Hydra side built to the same depth.

## Corrections to the Hydra spike

Made in `avatar-auth-spike.md` in the same commit, marked "Corrected" there:

- **"Every flow the design needs worked."** Wrong. The flows run worked, but child issuance, reuse with a live child or wait, the absolute deadline, RFC 8707 and `iss` on errors were not tested, and kkt was not running.
- **"Hydra does not support resource indicators."** Not established. That came from a subagent and was not tested. Ory's guide (updated 2026-11-05) documents RFC 8707 for self-hosted Hydra behind `feature_flags.resource_indicators_enabled`. That flag is not in the configuration schema of v26.2.0 or of the public repository's `master`.
- **Revoking kkt's record on an inactive introspection.** Wrong. Expiry also reads as inactive, and the answer names no grant or reason.
- **Caching introspection "because kkt's check makes revocation immediate".** Wrong for revocations Hydra makes itself.
- **"A modest amount of code".** That judgement was removed. The code was not built or measured.

## How to rerun

In `/workspace/kkt/spike-oauth-as`: `cargo test -- --nocapture --test-threads=1`. `tests/conformance.rs` runs the library's storage harness against the store. `tests/flows.rs` runs the scenarios above and prints what each observed.

## Sources

oauth-as

- Crate, versions, features: https://crates.io/crates/oauth-as and https://docs.rs/oauth-as/1.0.1
- Repository, README, CHANGELOG (1.0.1 loopback fix), PostgreSQL store: https://github.com/MattJackson/oauth-as
- Storage trait contract (take then write, barriers, sweep): `src/store.rs` in the 1.0.1 crate, https://docs.rs/oauth-as/1.0.1/oauth_as/store/trait.Storage.html
- Reuse handling on a failed revocation: `src/server.rs` in the 1.0.1 crate, around line 5668
- CIMD module and host duties: https://docs.rs/oauth-as/1.0.1/oauth_as/cimd/index.html
- Issues 25 and 26: https://github.com/MattJackson/oauth-as/issues/25 and https://github.com/MattJackson/oauth-as/issues/26
- OpenID Foundation listing: https://openid.net/certification/certified-fapi-2-0-op-security-profile-final-message-signing-final/

Specifications

- MCP authorization, 2026-07-28: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization
- CIMD draft: https://datatracker.ietf.org/doc/draft-ietf-oauth-client-id-metadata-document/
- RFC 8252 section 7.3 (loopback redirects): https://www.rfc-editor.org/rfc/rfc8252#section-7.3

Hydra corrections

- Ory audiences guide (RFC 8707), last updated 2026-11-05: https://www.ory.com/docs/hydra/guides/audiences
- Hydra configuration schema, v26.2.0 and `master`: https://raw.githubusercontent.com/ory/hydra/v26.2.0/spec/config.json and https://raw.githubusercontent.com/ory/hydra/master/spec/config.json

kkt

- `agent-credentials-design.md`, sections 5 to 7 and 11; `roadmap.md`, sections 12a and 13; `PRINCIPLES.md` and `CLAUDE.md` on `next` (storage, randomness, third-party types)
