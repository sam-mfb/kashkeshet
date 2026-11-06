# Multi-tenant kkt: planning

This branch holds the plan for running kkt as one server shared by many people and teams. Bug fixes and small features stay on `next`.

- `roadmap.md`: the plan, including the order of work (steps 1 to 6) and which crates each step touches (section 12a).
- `agent-credentials-design.md`: the settled design for how agents get their own narrow credentials in login mode.

`history/` holds the drafts, research and reviews that led to these, kept for reference:

- `multitenant-plan.md` and `multitenant-plan-review.md`: the first draft and its review.
- `roadmap-review.md` and `roadmap-changes.md`: the review of the full roadmap and the edits made from it.
- `agent-credentials-research.md` and `agent-credentials-research-sol.md`: two independent research reports on agent credentials.
- `agent-credentials-design-review-astra.md` and `agent-credentials-design-review-fable.md`: two reviews of the first credentials design.
- `creds-changes.md`: what changed when the credentials design was settled.
- `crates-changes.md`: what changed when the plan was fitted to the crates of 0.9.0, and Sam's decisions on the two new crates.
