# Multi-tenant kkt: planning

This branch holds the plan for running kkt as one server shared by many people and teams. Bug fixes and small features stay on `next`.

**The current plan is `plan-phase-1.md` and `plan-phase-2.md`** (2026-11-08). Where they differ from `roadmap.md`, `core-plus-two-products.md` or `project-split-study.md`, the plans win; the older files hold where the plans do not speak, and `roadmap.md`'s design sections (1 to 11, 13 and 14) remain the design of record for phase 2. `roadmap.md` will be revised to the plan's shape when phase 2 starts (Sam, 2026-11-08).

- `plan-phase-1.md`: phase 1: the fixed points (one kashkeshet repository and a private kkt-pro, Sam 2026-11-08; the swarm and the org in the core; one database file), the six steps that make today's crates the core with key mode unchanged, the rules that bind every step, what earlier documents this supersedes, and Sam's answers to the phase 1 questions.
- `plan-step-4-db.md`: phase 1 step 4 as Sam redesigned it on 2026-11-09: kkt-store over a database-neutral `kkt-db`, with `kkt-db-sqlite` its SQLite implementation; the query language, search, constraints, migrations, the baseline and the upgrade from 0.10.0, and the pull requests.
- `plan-phase-2.md`: phase 2: kkt-pro, from the additive core extensions through the roadmap's steps 3 to 6 on pro's own crates, with its open questions, deferred by Sam.
- `roadmap.md`: the plan, including the order of work (steps 1 to 6) and which crates each step touches (section 12a).
- `agent-credentials-design.md`: the settled design for how agents get their own narrow credentials in login mode.
- `avatar-auth-decision.md`: how kkt's authorization server for avatars is built (Hydra as a separate service, or oauth-as inside kkt), with the evidence, the blast radius of a flaw, and what is still open. Deferred by Sam.
- `project-split-study.md`: whether multi-tenant kkt could be a separate project on kkt's crates, as they are or restructured; recommends one repository with login mode as a setting, and says when a shared core would become possible.
- `core-plus-two-products.md`: the follow-up to the study: a shared `kkt-core` with the swarm as its concept, `kashkeshet` and `kkt-pro` as two products on it, Sam's decision on three repositories, the eight-step refactor that replaces the roadmap's steps 1 and 2, and what is still open (one file or one per swarm; the realm).

`history/` holds the drafts, research and reviews that led to these, kept for reference:

- `multitenant-plan.md` and `multitenant-plan-review.md`: the first draft and its review.
- `roadmap-review.md` and `roadmap-changes.md`: the review of the full roadmap and the edits made from it.
- `agent-credentials-research.md` and `agent-credentials-research-sol.md`: two independent research reports on agent credentials.
- `agent-credentials-design-review-astra.md` and `agent-credentials-design-review-fable.md`: two reviews of the first credentials design.
- `creds-changes.md`: what changed when the credentials design was settled.
- `crates-changes.md`: what changed when the plan was fitted to the crates of 0.9.0, and Sam's decisions on the two new crates.
- `admission-changes.md`: `kkt-admission` made the one place for every admission mode, and `kkt-login` renamed `kkt-oidc`.
- `avatar-auth-spike.md`: whether kkt's authorization server for avatars can be a separate service (Ory Hydra, Keycloak, Zitadel, Authentik), with Hydra tried hands-on. Research for a decision not yet made.
- `oauth-as-spike.md`: the Rust library oauth-as tried as kkt's authorization server inside kkt, with one SQLite store for its state and kkt's grants, against Astra's conditions; it also lists the corrections made to `avatar-auth-spike.md`.
- `keycloak-recheck.md`: Keycloak 26.8.2 looked at again, tried in Docker and read from its docs, source and issues; the reasons it was ruled out.
