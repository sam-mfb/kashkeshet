# What changed, 2026-11-06: admission modes and `kkt-oidc`

Editor: kkt-multitenant-planner. Approved by Sam, through sage-cardinal.

## The changes

- **`kkt-admission` owns how callers get in, in every mode.** Roadmap section 8.9 is new. It names three modes as parallel ways of doing one job: open, shared key (key mode) and OpenID Connect (login mode). `kkt-admission` defines, in one place, what every mode must provide: turning a presented credential into a `Principal`, and starting and ending a web session through a sign-in that is either a submitted form (key) or a redirect and callback (OpenID Connect).
- **The shared-key mode stays in `kkt-admission`.** A mode that wraps a library supplies its part from its own crate through that contract, and `kkt-server` connects them, as it does for `kkt-web`.
- **`kkt-web`'s `Gate` covers both sign-in shapes.** It says which shape the server uses, takes a typed key for a form, and gives a redirect address and finishes from the callback for OpenID Connect. Both end in a session or a refusal, so the pages see one contract whatever the mode.
- **`kkt-login` is renamed `kkt-oidc`** in `roadmap.md` and `agent-credentials-design.md`, including the crate table in 12a and the `CLAUDE.md` changes listed for step 3. Its scope and its `real` and `testing` features are unchanged. Step 3's entries for `kkt-admission` and `kkt-web` in 12a now name the mode contract and the redirect sign-in.

## Why

- Sam asked for parallel structure between key mode and login mode, so that each mode is one way of doing the same thing, defined in one place, and not a special case added beside the other.
- The key check stays in `kkt-admission` because a crate of its own would wrap nothing: it is a comparison of keys and a session, with no library behind it.
- `kkt-login` is renamed `kkt-oidc` so its name says what it is: the OpenID Connect client. "Login" also names the mode, which spans `kkt-admission`, `kkt-avatar-auth` and the rules, so it was a misleading name for one crate.

Nothing else in the design changed. `crates-changes.md` keeps the old name; history files record what was true when they were written.
