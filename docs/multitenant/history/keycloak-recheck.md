# Keycloak, looked at again

Status: research, 2026-11-06, by kkt-authspike-2, at Sam's request (#kkt-dev, message 689). The first spike (`avatar-auth-spike.md`) set Keycloak aside quickly; this looks again, properly, at current Keycloak. Sam has since ruled it out (`../avatar-auth-decision.md`, section 2.3). Nothing in kkt was changed.

## Summary

Keycloak 26.8.2 handles the standard protocol well:

- it sends `iss` on success and error redirects;
- it answers `slow_down` to fast polling;
- reusing a refresh token ended the whole grant in my test;
- CIMD support is coming.

It fits kkt badly where the design depends on it:

- **The pages.** kkt cannot own the consent or device-approval pages without Java plugins on interfaces Keycloak calls internal. There is no REST equivalent of Hydra's way for an app to accept a login or approve a device code.
- **The grant record.** Keycloak keeps its own users, sessions and consents, so kkt's grant record cannot be the authority without a second store kept in step. Keycloak has no per-grant ID and no cascade to child grants.
- **Child avatars.** There is no supported way to issue a child avatar's credentials.
- **Experimental features.** CIMD and resource indicators are both experimental, with open bugs whose fixes are on the 27.0 branch only.
- **Shipping it.** It is the heaviest option: a JVM, a server database such as Postgres, and about 1.25 GB of memory per instance.

## What I ran

Keycloak 26.8.2 (released 2026-11-05) in Docker, in development mode with the `cimd` and `resource-indicators` features on, and a Python script on the same Docker network. The script:

1. created a realm with "Revoke Refresh Token" on and maximum reuse 0;
2. created a user with the `offline_access` role, a public client for the authorization code grant (PKCE S256) and a public client for the device grant;
3. read the metadata;
4. triggered error redirects;
5. logged in through the login form and redeemed the code;
6. refreshed, then reused the old refresh token;
7. polled the device endpoint quickly;
8. tried anonymous dynamic registration;
9. sent a URL as client ID with no CIMD policy configured.

I also started the server in production mode with the built-in file database. The containers were removed afterwards. A research subagent read the documentation, the source at tag 26.8.2, and the issue tracker. Its findings are marked "read" below, and anything it could not confirm is said to be unconfirmed.

## Findings

### CIMD

- **Status.** Experimental, behind `--features=cimd`; the docs warn of breaking changes. Tried: the metadata advertises `client_id_metadata_document_supported: true`, and with no policy configured a URL client ID gets an error page.
- **How it works (read).** It is configured through client policies. A `client-id-uri` condition matches a URL client ID on a trusted host, and a `client-id-metadata-document` executor fetches the document.
  - Only `https` URLs are accepted, with no query string, fragment or user info.
  - The cache lasts from 300 seconds to 3 days.
  - Documents are capped at 5000 bytes.
  - Trusted domains also apply to the URLs inside the document, and an empty list denies everything.
  - It stores a Keycloak client for each URL.
  - Which draft revision it follows is unclear: the docs say -01, and issues mention -02 and -03.
- **Open issues (read):**
  - **Server-side request forgery and client takeover**, reproduced on 26.8.0: #53812. The fix (PR #53912), which refuses private addresses, is on the main branch for 27.0, not confirmed in 26.8.x. Reviewers note that DNS rebinding remains and that first registration is still unauthenticated.
  - **Unknown fields in a document cause rejection**, which affected Codex's and ChatGPT's documents: #51236. Fixed on main; a backport to 26.8 was requested but is not confirmed.
  - **PKCE is not enforced on CIMD clients:** #52795, open.
  - **The same-domain check skips extra redirect URIs:** #50748, open.
  - **A null document crashes it:** #54323, open.
  - **Following draft -03:** #54668, unassigned.
- **Timeline (read).** The epic #50242 is on the 27.0 milestone, due 2027-03-31 and about 5% done when last updated. The issue to promote it to preview, #47765, had 11 of 34 sub-issues done. No date is given for preview or full support.

### The pages

Keycloak has no equivalent of Hydra's challenge API. An outside app cannot accept a login, supply a consent decision, or approve a device code over REST (read). The ways to get kkt's pages:

- **Themes.** FreeMarker templates, presentation only. Showing kkt's data, such as the avatars a human may pick, needs Java. The docs say templates may need updating on upgrade, and a template can run code inside the server (read).
- **A custom authenticator or action-token handler.** These are Java JARs dropped into Keycloak. The server development guide calls these SPIs internal: "any new release of Keycloak can change it without prior notice" (read). kkt would ship Java and rebuild and retest it for each Keycloak version.
- **Redirecting out to kkt and back.** There is no official pattern; it would be a custom Java authenticator (read).
  - One route I did not try is identity brokering. Keycloak would send the human's login to kkt as an outside OpenID Connect provider, and kkt would pick the avatar there.
  - That makes kkt an OpenID Connect issuer anyway, and Keycloak still creates its own user and session for each avatar.
- **Device approval.**
  - The user-code page is Keycloak's own and can only be restyled.
  - Approval happens inside Keycloak's browser flow, with no outside approval call (read, `DeviceEndpoint` and `DeviceGrantType`).
  - The device-grant design document says consent is always shown for device logins. Not checked on 26.8.2.

### kkt's grant as the authority

- **Approval is two stores.** Keycloak keeps users, sessions and consents in its own database, so approval cannot be written in one transaction with kkt's grant. That is the same two-store problem as Hydra.
- **Tokens.** Access tokens are JWTs (tried). Keycloak has no opaque tokens; the closest is "lightweight" tokens, and introspection is for confidential clients only (read). kkt can still check its own grant on each request if the token names the grant. Putting the grant ID in a token through a user or session note and a protocol mapper is plausible but untested.
- **Revocation (read).** `DELETE /admin/realms/{realm}/users/{id}/consents/{client}` revokes a user's consent and offline tokens for one client. There is no per-grant ID and no cascade, so children and waits remain kkt's job, kept in step with Keycloak.

### The rest of the design's list

- **`iss` (RFC 9207), tried.** Present on success and on every error redirect triggered: a `plain` PKCE challenge, an unknown scope, and a disallowed response type. The metadata advertises it. Issue #25584 reports it missing on `prompt=none` errors; I did not check whether that is fixed.
- **`slow_down`, tried.** The second and third immediate polls got `slow_down`. The interval is set per realm.
- **Refresh reuse, tried.** This ran on an offline session with "Revoke Refresh Token" on and maximum reuse 0.
  - Reusing the old refresh token gave `invalid_grant` ("Maximum allowed refresh token reuse exceeded").
  - The newest refresh token was then refused too ("Session doesn't have required client").
  - The user's offline sessions for that client dropped to zero.

  So in this test the whole grant ended. The subagent's reading of `TokenManager` suggested it might not, so this deserves a second test if Keycloak is reconsidered. Issue #49213 (open) is a separate bug: a database error during refresh can end a session for good.
- **Absolute deadline (read).** "Offline Session Max" with "Offline Session Max Limited" caps offline tokens from when the session was created, per realm or per client. It is not per grant, so a child whose deadline is its parent's would need kkt's own check. Not tested.
- **Resource indicators (RFC 8707).** Experimental.
  - Tried: a `/mcp-admin` resource at the authorization endpoint was not refused; the login page was shown. A resource at the token endpoint got `invalid_target`, because I had configured no resource.
  - Open bugs (read): #53261, an offline refresh ignores `resource` and widens the audience; #54296, introspection reports the resource server's own tokens as inactive. Both are fixed only on main.
- **Child credentials (read).** No supported route.
  - Standard token exchange (v2) cannot act as another subject and cannot issue offline tokens.
  - Delegation, in preview, issues a token naming the actor with no refresh token.
  - The approach used in the oauth-as spike, a device grant kkt approves itself, does not carry over, because Keycloak approves device codes only in its browser flow.
- **Dynamic registration, tried.** Anonymous registration was refused by the default "Trusted Hosts" policy. It can be configured.

### Running it

- **Size, tried.** The image is 750 MB uncompressed. Memory was about 840 MB idle in development mode; the sizing guide says 1250 MB per instance (read). It starts in about 5 seconds.
- **Database.** The docs say: "The dev-file database is not suitable for production use-cases, and must be replaced before deploying to production" (read). Production needs PostgreSQL 14 to 18 or another server database. Tried: `start --db=dev-file` does start in production mode without refusing, but that is unsupported.
- **Upgrades (read).** Rolling updates are not supported across some patch changes; 26.8.2 needs a full restart of every instance. Custom Java providers must be rebuilt for each version. Whether the database migrates automatically on start was not confirmed from the docs.
- **For `kkt setup` and Docker.** Two more containers (Keycloak and its database) and roughly 1.5 GB of memory between them, Java plugins to build and pin, two databases to back up and restore together, and the same two-store synchronisation as Hydra. A launchd or systemd install would need a JVM and a server database on the machine.
- **Licence.** Apache-2.0.

## Verdict

Keycloak is the strongest of the candidates on standard protocol features, and it has a large team and a long history. But it fits kkt worse than Hydra on the two things the design depends on:

- **The pages:** kkt would need Java plugins on interfaces Keycloak calls internal to own the consent and approval pages.
- **The grant record:** kkt's grant record cannot be the authority without keeping two stores in step.

It is worse than oauth-as on those two points, and also on child credentials and per-grant deadlines. It is the heaviest to ship.

It would become the best choice only if one of these held:

- kkt gave up having its own pages pick the avatar, and accepted Keycloak's records as the authority, with kkt mirroring them;
- kkt ran only as a Docker deployment with a server database;
- Keycloak shipped CIMD and resource indicators as supported features and added an API for an outside app to supply consent and approve device codes. I found no such API on any roadmap.

## Sources

Keycloak documentation

- Keycloak as an MCP authorization server (CIMD, resource indicators): https://www.keycloak.org/securing-apps/mcp-authz-server
- Themes: https://www.keycloak.org/ui-customization/themes
- Server development guide (SPIs, internal status): https://www.keycloak.org/docs/latest/server_development/index.html
- Server administration guide (session lifetimes, application-initiated actions): https://www.keycloak.org/docs/latest/server_admin/index.html
- Token exchange: https://www.keycloak.org/securing-apps/token-exchange
- Databases: https://www.keycloak.org/server/db
- Memory and CPU sizing: https://www.keycloak.org/high-availability/single-cluster/concepts-memory-and-cpu-sizing
- Upgrading: https://www.keycloak.org/docs/latest/upgrading/index.html
- Release notes: https://www.keycloak.org/docs/latest/release_notes/index.html

Keycloak source at tag 26.8.2

- `DeviceEndpoint.java`: https://github.com/keycloak/keycloak/blob/26.8.2/services/src/main/java/org/keycloak/protocol/oidc/grants/device/endpoints/DeviceEndpoint.java
- `DeviceGrantType.java`: https://github.com/keycloak/keycloak/blob/26.8.2/services/src/main/java/org/keycloak/protocol/oidc/grants/device/DeviceGrantType.java
- `TokenManager.java`: https://github.com/keycloak/keycloak/blob/26.8.2/services/src/main/java/org/keycloak/protocol/oidc/TokenManager.java
- `UserResource.java` and `UserConsentManager.java` (revocation): https://github.com/keycloak/keycloak/blob/26.8.2/services/src/main/java/org/keycloak/services/resources/admin/UserResource.java and https://github.com/keycloak/keycloak/blob/26.8.2/services/src/main/java/org/keycloak/services/managers/UserConsentManager.java
- `OIDCLoginProtocol.java` (`iss`): https://github.com/keycloak/keycloak/blob/26.8.2/services/src/main/java/org/keycloak/protocol/oidc/OIDCLoginProtocol.java
- Licence: https://github.com/keycloak/keycloak/blob/26.8.2/LICENSE.txt

Keycloak issues

- CIMD epic and preview: https://github.com/keycloak/keycloak/issues/50242 and https://github.com/keycloak/keycloak/issues/47765; 27.0 milestone: https://github.com/keycloak/keycloak/milestone/71
- CIMD security and bugs: https://github.com/keycloak/keycloak/issues/53812 (fix https://github.com/keycloak/keycloak/pull/53912), https://github.com/keycloak/keycloak/issues/51236, https://github.com/keycloak/keycloak/issues/52795, https://github.com/keycloak/keycloak/issues/50748, https://github.com/keycloak/keycloak/issues/54323, https://github.com/keycloak/keycloak/issues/54668
- Resource indicators: https://github.com/keycloak/keycloak/issues/53261 and https://github.com/keycloak/keycloak/issues/54296
- Refresh reuse after a database error: https://github.com/keycloak/keycloak/issues/49213
- `iss` on `prompt=none` errors: https://github.com/keycloak/keycloak/issues/25584
- Outside consent screens, unresolved: https://github.com/keycloak/keycloak/discussions/15050 and https://github.com/keycloak/keycloak/discussions/23139
- Device grant design: https://github.com/keycloak/keycloak-community/blob/main/design/oauth2-device-authorization-grant.md

kkt

- `../avatar-auth-decision.md`, `avatar-auth-spike.md`, `oauth-as-spike.md`; `../agent-credentials-design.md`, sections 4, 5, 7 and 11. #kkt-dev messages 689, 696, 697 and 698.
