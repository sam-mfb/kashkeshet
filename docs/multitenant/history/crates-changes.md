# What changed, 2026-11-05: fitting the plan to the crates

Editor: kkt-multitenant-planner. Checked against `main` at `cb3a827` (0.9.0).

0.9.0 split kkt into 27 crates under `crates/` (#247). `CLAUDE.md` now lists each crate and what it owns, and `PRINCIPLES.md` gives the rules for splitting code among them. The plan was written before that and named modules that have since moved. The design itself is unchanged: no noun, rule, surface, step or test changes. Only where the work lands, and one ordering of work, changed.

## Sam's decisions

Asked in #kkt-dev and answered through sage-cardinal on 2026-11-05.

- **kkt's own authorization server for avatars gets its own crate**, `kkt-avatar-auth`. `CLAUDE.md` limits axum to `kkt-web`, `kkt-mcp` and `kkt-server`; that rule is widened to this fourth crate when step 3 lands. Whether it wraps a library or configures a sidecar stays open (roadmap, section 13).
- **The mock authorization server is a dev-only crate**, `kkt-mock-issuer`, which `tests/common` starts in-process. The plan had it as a development binary beside the web preview example, but tests cannot link an example binary, and a crate that is only a dev-dependency stays out of the release just as `kkt-scratch` does.

## roadmap.md

- **New section 12a, "Where the work lands."** It lists the three new crates (`kkt-login`, `kkt-avatar-auth`, `kkt-mock-issuer`) with what each owns, must not do and its features, then which existing crates and parts of the program each step touches, and the `CLAUDE.md` changes each step brings.
- **The `login/` part is now the `kkt-login` crate** (8.5), with the OpenID Connect and JWT libraries behind `real` and the fake `TokenVerifier` behind `testing`, per `PRINCIPLES.md` rule 8.
- **Carrying the caller's identity from admission to the rules moved from step 3 to step 2.** It is now the one change that crosses `kkt-admission`, `kkt-web`, `kkt-server`, `kkt-mcp` and `kkt-rules`. Doing it in step 2, where every caller is still the one key-mode user, keeps it invisible, and step 3 then adds only a new kind of caller. Step 2 lands as three pull requests, each mostly in one crate (rule 9). Step 3 says how a login-mode avatar token passes admission: `kkt-admission` must not judge agent tokens, so it passes the token on and `kkt-rules` checks the grant.
- **Moved code renamed.** `rules::access::decide` and `rules::roles::allows` are now in `kkt_rules`; `events.rs` is `kkt-events`; `config.rs` is `kkt-config`; `format` is `kkt-format`; `provenance` is `kkt-provenance`'s `settings_text`; migrations are in `crates/kkt-store/migrations/` and tested in `tests/migrations.rs`; the instructions and tool text are in `crates/kkt-mcp/`. Where the plan cites today's code (`agent_view`, `admin_agent`, `overview`, `admit`, `AdminServer`, `RulesSource`), it now names the crate.
- **The migration test starts from 0.9.0.** 0.9.0's schema is 0019, the same as 0.8.0's, so the populated test database is the same; new migrations number from 0020.
- **Smaller notes:** the Host check is `kkt-hosts`; login settings are gathered by `kkt-settings` and checked by `kkt-config`; the bootstrap line is decided by `kkt-setup` and written by the program; `--org` lands in the program's `cli.rs` and its `lanes` and `agent` commands; `/mcp-user` is a third endpoint in `kkt-mcp` with its own tool text, instructions and contract snapshot.

## agent-credentials-design.md

- Names the crates: `kkt-names`, `kkt-web`'s `change.rs`, `kkt-config`, `crates/kkt-mcp/agent-instructions.md`, `kkt-rules`' `auth.rs`.
- Section 11 says the authorization server lives in `kkt-avatar-auth`, which decides nothing about approval and reaches the rules through a contract that `kkt-server` adapts, and that library or sidecar is still open. Sign-in with the provider is the `kkt-login` crate.

## What the reorganization made easier or harder

- **Easier.** `kkt-admission` was written to take a bearer-token variant. `kkt-web`'s `Source` and `Gate` contract means the per-user web page is a new adapter in `kkt-server`, not a rewrite of the pages. Limits, settings and their record each sit in one crate. Only `kkt-store`'s `sqlite.rs` holds SQL, so "query by the capability's scope, never fetch wider" can be held at `Reads` and `Writes`.
- **Harder.** Steps 1, 2 and 3 cross many crates, so each is split into pull requests that stay mostly in one. The OAuth, OpenID Connect and JWT libraries must each be wrapped in one crate and added to `.github/third-party-features.txt`, and CI's per-crate, per-feature build checks the new crates' features.
- **Unnecessary.** No step goes away.

The `history/` files keep the old paths they cite; they record what was true when they were written.
