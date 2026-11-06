# Multi-tenant kkt: planning

This branch holds the plan for running kkt as one server shared by many people and teams. Bug fixes and small features stay on `next`.

- `roadmap.md`: the plan, including the order of work (steps 1 to 6) and which crates each step touches (section 12a).
- `agent-credentials-design.md`: the settled design for how agents get their own narrow credentials in login mode.
- `avatar-auth-decision.md`: how kkt's authorization server for avatars is built (Hydra as a separate service, or oauth-as inside kkt), with the evidence, the blast radius of a flaw, and what is still open. Deferred by Sam.

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
