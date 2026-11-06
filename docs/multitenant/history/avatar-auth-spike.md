# Spike: a separate authorization server for avatars

Status: research for a decision Sam has not made, 2026-11-06, by kkt-authspike. It answers the open point in `roadmap.md` section 13: can kkt's authorization server for avatars (`agent-credentials-design.md`, section 11) be a separate service rather than a library inside kkt? Nothing in kkt was changed.

## Summary

**A separate service works, and Ory Hydra is the one to use.** Hydra was built for this split: it handles only the OAuth protocol, and the app supplies every page and tells Hydra, through an admin API, what the human decided. I ran Hydra v26.2.0 and played kkt's part through that API. Every flow the design needs worked: dynamic registration, authorization code with PKCE, the device grant with kkt's own approval page, opaque tokens with kkt's grant ID inside, refresh rotation where reuse kills the whole grant, and revoking one grant on demand. Hydra is small (a 70 MB image, about 30 to 60 MB of memory), can keep its data in SQLite, ships single binaries for Linux and macOS, and is Apache-2.0.

**The gaps are the two things the current MCP spec added.** The MCP authorization spec of 2026-07-28 says authorization servers SHOULD support Client ID Metadata Documents (CIMD) and SHOULD send `iss` in the authorization response (RFC 9207). It deprecates dynamic registration and keeps it only for compatibility. Open-source Hydra does neither:

- **CIMD** is coming to Hydra, but Ory said on 2026-10-26 that it will ship only in their paid Enterprise build and Ory Network, and that they will not accept a community version. kkt can fill the gap: it fetches the client's metadata document itself and registers the client in Hydra with the URL as its client ID. I tried the registration half; Hydra accepts a URL as client ID from the admin API.
- **`iss`** is not sent and not advertised. kkt can fill this gap by serving Hydra's public endpoints through itself, adding `iss` to the redirect, and serving the metadata document itself.

So kkt would not just configure Hydra. It would stand in front of Hydra's public side, serving its own metadata, adding `iss`, handling CIMD, enforcing `slow_down` and rate limits (Hydra's polling interval is advice only), and attaching the requester's suggested name to device requests. That is a modest amount of code in `kkt-avatar-auth`, but it is protocol code, the thing a sidecar was meant to spare kkt. Sam should weigh it against a library inside kkt.

Other gaps and costs:

- **Two stores.** Hydra keeps its own consent sessions and tokens. kkt keeps its grant record and checks it on every request. kkt has to revoke in both, and Hydra does not tell kkt when it revokes a grant itself after a refresh token is reused.
- **Hydra's admin API has no authentication.** Anything that can reach it can approve grants. It must only be reachable by kkt: a private Docker network, or a Unix socket with file permissions in a service install.
- **Hydra's open-source releases are slow**: v2.3.0 (January 2025), v25.4.0 (November 2025), v26.2.0 (March 2026), nothing since. The device grant arrived in v25.4.0.
- **Every install grows a second program.** The Docker install gains a container. The launchd and systemd install gains a second binary and service to download, run and upgrade together with kkt, which cuts against design principle 1.

**Keycloak** supports more of the protocol out of the box (CIMD as an experimental feature, `iss`, the device grant) but does not fit. Its login, consent and device pages are its own; changing them means themes or Java plugins, not kkt's pages. It needs Postgres or another server database and about 1.25 GB of memory. **Zitadel** lets an app supply its own login pages, but it keeps the grants in its own event store, needs Postgres, has no CIMD or `iss` yet, and moved to AGPL in v3. **Authentik** builds its pages from its own flows, needs Postgres and 2 GB, and has CIMD only on its roadmap for 2027.2.

## Candidates against questions 1 to 5

"Tried" means I ran it against Hydra v26.2.0. Everything else is from the sources listed at the end.

| | Ory Hydra v26.2.0 | Keycloak 26.8.2 | Zitadel v4.20.1 | Authentik 2026.8.5 |
|---|---|---|---|---|
| 1. Device grant (RFC 8628) | Yes, since v25.4.0. Tried. | Yes, since 13.0.0. | Yes, since about v2.24 (2023); release not confirmed. | Yes; since which version not confirmed. |
| 2. Dynamic registration (RFC 7591) | Yes, off by default. Open to anyone once enabled; the server picks the client ID. Tried. Not listed in the metadata until `webfinger.oidc_discovery.client_registration_url` is set. | Yes. Anonymous registration is allowed only from trusted hosts (none by default), otherwise with an initial access token. | Yes, since July 2026 (PR 12313); off by default, open or token-only. Release not confirmed. | Yes, since 2026.8.0, per provider, needs a token with a special scope. The release notes and blog disagree on whether it is an Enterprise feature. |
| 3. CIMD | No. Planned for the paid Enterprise build only (issue 4061, 2026-10-26). kkt can register the client itself through the admin API with the URL as client ID; tried. | Experimental since 26.6.0 (`--features=cimd`), with open hardening issues; promotion targeted at 27.0. | No. Two open pull requests (12316, 12860), not merged. | No. Planned for 2027.2.0 (issue 27431), possibly Enterprise only. |
| 4. `iss` in the authorization response (RFC 9207) | No: the redirect carries only `code`, `scope`, `state`, and the metadata does not advertise it. Tried. | Yes. | Not found in the code; not confirmed. | Not found in the code; not confirmed. |
| 5. Refresh rotation, reuse revokes the grant | Yes, on by default (`oauth2.grant.refresh_token.rotation_grace_period: 0s`). Reusing a rotated token returned `invalid_grant` and made the newest access and refresh tokens inactive too. Tried. | Rotation yes (`Revoke Refresh Token`, `refreshTokenMaxReuse`). Whether reuse ends the whole session is not confirmed; an open bug (49213) suggests it is inconsistent. | Rotation yes. Reuse detection not found; not confirmed. | Rotation yes (`refresh_token_threshold: 0`). Reuse detection not confirmed. |

All four support PKCE S256 and authorization server metadata (RFC 8414). For Hydra, both `/.well-known/oauth-authorization-server` and the OpenID Connect discovery path answered.

## 6. Fit with kkt's design

**Pages.** Hydra never shows the human a page. For each request it redirects the browser to an address kkt configures, carrying a one-time "challenge". kkt shows its page, then tells Hydra the outcome through the admin API, and Hydra replies with where to send the browser next. There are three such steps: login (kkt names the subject), consent (kkt grants scopes and attaches its own data to the tokens) and, for the device grant, code entry. All three pages can be kkt's, in `kkt-web`, under its existing rules for form posts. What I saw:

- Authorization code: the browser goes Hydra, then kkt's login page, back to Hydra, then kkt's consent page, back to Hydra, then to the client with the code. kkt already knows who the human is from the page session, so it can accept the login step at once without showing anything. The human sees one page, the consent page.
- Device grant: the `verification_uri` Hydra returns is Hydra's own, which redirects to kkt's page with a challenge. kkt's page takes the typed user code and sends it to Hydra. A wrong code was refused. The right one led on through login and consent, then to kkt's "done" page. The next poll returned the tokens; a second redemption of the device code was refused.
- Hydra's device request takes only `client_id` and `scope`. The `kkt` command's suggested handle and purpose need another path. If kkt serves the device endpoint itself and passes the request on, it can store the suggestion against the user code that Hydra returns.
- Polling faster than the interval returned `authorization_pending`, not `slow_down`. Hydra's documentation calls the interval informational. kkt's rule (design 4.3) would be enforced in kkt.
- Hydra has no call to reject a device code. kkt can only let it expire, or refuse at the consent step.

**The grant store.** Hydra keeps its own records of each consent and its tokens, so there are two stores. The design still holds if kkt's grant record stays the authority:

- At consent, kkt puts its grant ID in the token data (`session.access_token`). Introspection returned it (`ext.kkt_grant`), so every request at `/mcp` leads from token to kkt's grant record, which kkt checks as the design says.
- Revoking one grant: Hydra gives each consent an ID (`consent_request_id`). Revoking by that ID alone ended that grant's access and refresh tokens and left another grant of the same subject and client working. Passing the subject as well was refused with a 400. Revoking everything for a subject also worked.
- kkt revokes its own record first, which takes effect at once, then Hydra's. If the Hydra call fails, the tokens it still holds are refused by kkt's check anyway, and kkt retries.
- When Hydra itself kills a grant because a refresh token was reused, kkt is not told. The avatar stops working (its tokens are dead), but kkt's pages would still show the grant as active until kkt notices. kkt could list the subject's consents from Hydra when showing the page, or revoke its record when it sees an introspection of an inactive token for a grant it thinks is active. This needs a decision.
- Hydra can call kkt on every token issue or refresh (`oauth2.token_hook`). That would let kkt refuse a refresh for a grant it has revoked. I did not try it.

**Checking each token.** Introspection over the admin API took about 4 ms per call (200 calls in a row, Hydra on SQLite, both in Docker on one machine). That is one HTTP call plus one database read per `/mcp` request. kkt could cache the result for a few seconds, since its own grant check is what makes revocation immediate. Another option is JWT access tokens (`strategies.access_token: jwt`), which kkt can check without calling Hydra, but they give up the design's opaque tokens.

**Audience.** Hydra does not support resource indicators (RFC 8707). It has its own `audience` parameter, which kkt's consent step can set to `/mcp`.

## 7. Running it

| | Hydra | Keycloak | Zitadel | Authentik |
|---|---|---|---|---|
| Database | Its own: SQLite, Postgres, MySQL or CockroachDB. SQLite worked in the standard image. | Its own server database (Postgres, MySQL, MariaDB, SQL Server, Oracle); no SQLite. | Postgres. | Postgres. |
| Memory | 32 MB measured by Docker after the test run (56 MB resident). | About 1.25 GB per instance, by its sizing guide. | About 512 MB. | Host minimum 2 GB. |
| Size | 70 MB image; 14 to 16 MB binaries for Linux and macOS, with or without SQLite. | About 250 to 280 MB image (third-party figures). | Not confirmed. | Not confirmed. |
| Licence | Apache-2.0; CIMD will be in the paid build only. | Apache-2.0. | AGPL-3.0 since v3.0.0. | MIT, with a separate licence for the Enterprise folder. |

How kkt would ship Hydra:

- **Docker install.** A second service in each Compose file, on a network shared only with kkt: public port reached through kkt, admin port never published. A volume for the SQLite file. `kkt setup` writes Hydra's configuration and its system secret (a key of the same kind kkt already manages). `kkt upgrade` moves both images together and runs `hydra migrate sql up` before starting Hydra. `kkt backup` must copy Hydra's database as well. These change the Compose files and managed folder that existing installs depend on, which design principle 3 covers, but they matter only in login mode; key-mode installs need not get Hydra at all.
- **Service install (launchd, systemd).** A second binary, downloaded and checked against Ory's signed checksums, in a version kkt pins; a second service file; its admin API on a Unix socket with owner-only permissions (Hydra's serve options include a socket mode; not tried). This is the larger cost.
- **Configuration.** The issuer is kkt's public URL. The login, consent and device addresses are kkt's pages. Access tokens are opaque, refresh grace period 0, dynamic registration on, device polling interval and code lifetime from kkt's `Limits`. Hydra is started with these, as kkt's settings, never edited by hand.

## 8. Testing

- **Most tests use a fake.** `kkt-avatar-auth` states what it needs from the authorization server in a contract of its own: start a request, accept login, accept consent with a grant ID, accept a device code, introspect, revoke one grant, revoke a subject, register a client. A fake in its `testing` feature keeps these in memory, as `kkt-oidc`'s fake `TokenVerifier` does. Rule tests, page tests and most end-to-end tests run against it. The fake must reproduce the behavior the design depends on: refresh reuse kills the grant, a device code works once, and revoking one consent leaves the others.
- **A smaller set runs against real Hydra.** These tests start the Hydra binary (pinned version, SQLite in the test's own scratch folder under `target/tmp`) on a free port, as `tests/common/serving.rs` starts kkt, with the admin API on a Unix socket. They run the flows tried in this spike through kkt's real pages: dynamic registration, authorization code with consent, the device grant with approval, refresh rotation and reuse, revoking one grant, and CIMD registration. They also check the gaps kkt fills: `iss` on the redirect and `slow_down`. CI downloads the binary for Ubuntu and macOS and checks it against Ory's checksums.
- **Upgrading Hydra** is then a change to the pinned version that must pass that set. Hydra's own migrations need a test, as kkt's do: a Hydra database from the previous pinned version must still work after `migrate sql up`.

## What I ran

Hydra v26.2.0 in Docker (`oryd/hydra:v26.2.0`, `--dev` for plain HTTP), SQLite, opaque tokens, refresh grace period 0, dynamic registration on. A Python script on the same Docker network played the client and kkt's pages, using only Hydra's public and admin APIs. It ran: metadata; anonymous dynamic registration; authorization code with PKCE S256 and consent carrying a grant ID; introspection, timed; refresh, then reuse of the old refresh token; three grants and revocation of one of them by consent ID; the device grant with a wrong and a right user code, then double redemption and fast polling; a URL as client ID at the authorization endpoint, then registered through the admin API and used again; revocation of everything for one subject. The containers were removed afterwards. I did not run Keycloak, Zitadel or Authentik, and I did not try Hydra's token hook, Unix socket or Postgres.

## Sources

MCP specification

- Authorization, 2026-07-28: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization
- Client registration, 2026-07-28 (DCR deprecated, CIMD preferred): https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration

Ory Hydra

- Releases (v25.4.0 device grant and RFC 8414; v26.2.0 assets): https://github.com/ory/hydra/releases
- Configuration schema, v26.2.0: https://raw.githubusercontent.com/ory/hydra/v26.2.0/spec/config.json
- API, v26.2.0: https://raw.githubusercontent.com/ory/hydra/v26.2.0/spec/api.json
- Device authorization guide: https://www.ory.com/docs/oauth2-oidc/device-authorization
- Graceful token refresh (grace period, reuse detection): https://www.ory.com/docs/hydra/guides/graceful-token-refresh
- Databases: https://www.ory.com/docs/hydra/self-hosted/dependencies-environment
- Client handler (DCR overwrites the client ID; admin create keeps it): https://raw.githubusercontent.com/ory/hydra/master/client/handler.go
- CIMD tracking issue, Enterprise-only statement of 2026-10-26: https://github.com/ory/hydra/issues/4061 and https://github.com/ory/hydra/issues/4134
- CIMD documentation pull request (unmerged): https://github.com/ory/docs/pull/2394
- Serve options including Unix socket mode: https://raw.githubusercontent.com/ory/x/master/configx/serve.schema.json
- Licence: https://github.com/ory/hydra/blob/master/LICENSE
- A third-party project fronting Hydra for MCP, reporting no `iss`: https://github.com/rado0x54/ShellWatch/issues/242 and https://github.com/rado0x54/ShellWatch/pull/254

Keycloak

- Releases: https://github.com/keycloak/keycloak/releases
- As an MCP authorization server (support table: RFC 9207 supported, CIMD and RFC 8707 experimental): https://www.keycloak.org/securing-apps/mcp-authz-server
- 26.6.0 release (CIMD): https://www.keycloak.org/2026/04/keycloak-2660-released
- CIMD epic and issues: https://github.com/keycloak/keycloak/issues/50242, https://github.com/keycloak/keycloak/issues/51236, https://github.com/keycloak/keycloak/issues/53812
- Client registration: https://www.keycloak.org/securing-apps/client-registration
- Device grant, introspection: https://www.keycloak.org/securing-apps/oidc-layers and https://www.keycloak.org/docs/latest/release_notes/index.html
- Refresh token reuse code and bug: https://raw.githubusercontent.com/keycloak/keycloak/main/services/src/main/java/org/keycloak/protocol/oidc/TokenManager.java and https://github.com/keycloak/keycloak/issues/49213
- Databases: https://www.keycloak.org/server/db
- Memory sizing: https://www.keycloak.org/high-availability/single-cluster/concepts-memory-and-cpu-sizing

Zitadel

- Releases: https://github.com/zitadel/zitadel/releases
- Device grant: https://github.com/zitadel/zitadel/pull/5646 and https://zitadel.com/docs/guides/integrate/login-ui/device-auth
- Dynamic registration: https://github.com/zitadel/zitadel/pull/12313
- CIMD pull requests: https://github.com/zitadel/zitadel/pull/12316 and https://github.com/zitadel/zitadel/pull/12860
- Endpoints: https://zitadel.com/docs/apis/openidoauth/endpoints
- Custom login: https://zitadel.com/docs/guides/integrate/login-ui/oidc-standard
- Running in production: https://zitadel.com/docs/self-hosting/manage/production
- Licence change: https://zitadel.com/blog/apache-to-agpl

Authentik

- Releases: https://github.com/goauthentik/authentik/releases
- OAuth2 provider: https://docs.goauthentik.io/add-secure-apps/providers/oauth2/
- Dynamic registration: https://docs.goauthentik.io/add-secure-apps/providers/oauth2/dynamic-client-registration/ and https://docs.goauthentik.io/releases/2026.8/
- CIMD tracking issue: https://github.com/goauthentik/authentik/issues/27431
- Authorization and token code: https://github.com/goauthentik/authentik/blob/main/authentik/providers/oauth2/views/authorize.py and https://github.com/goauthentik/authentik/blob/main/authentik/providers/oauth2/views/token.py
- Architecture and requirements: https://docs.goauthentik.io/core/architecture/ and https://docs.goauthentik.io/install-config/install/docker-compose/
- Licence: https://github.com/goauthentik/authentik/blob/main/LICENSE
