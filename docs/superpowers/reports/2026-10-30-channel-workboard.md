# Read-only workboard implementation

Implemented #B in kkt-dev on branch codex/workboard-readonly. Worktree:
 /workspace/target/worktrees/workboard-readonly.

## Delivered behavior

| Area | Result |
|---|---|
| Channel and aggregate views | Work navigation, channel selection, pending requests, tasks, PRs/issues, history filter |
| Agent maintenance | Version 1 reports through existing messages; revisions, retries, conflicts, evidence, requester-reported external resolution |
| Refresh | Existing SSE invalidation, snapshot-race handling, reconnect/focus recovery, 30-second visible-page reconciliation |
| Trust and freshness | Read-only routes, no human approval writer, explicit attribution, exact-revision verification, stale/failed checks, bounded history errors |
| Compatibility | No migration, new MCP tools, credentials, provider fetcher, or agent runner |

Documentation: docs/workboard.md. The design, plan, and unpublished upstream
proposal remain under docs/superpowers. All development and checks ran in
the kkt-dev container.

## Validation

The final full Rust suite passed 872 tests across 26 test targets, including
36 focused workboard tests. Cargo Clippy with all targets and warnings denied,
Cargo formatting checks, and diff whitespace checks passed. This suite
includes the existing MCP contract, migration, backup, admin, and web-login
checks without updating their public-contract fixtures.

Both browser scripts passed against a fresh preview using --seed 7 --rate 1
--page-messages 10. The existing pagination script needs at least several
pages in its first eligible channel; larger pages can make that fixture
assertion fail. The workboard script covers snapshot races, invalidation,
out-of-order fetches, reconnect, missed events, focus, history, same-revision
semantic updates, and honest event-stream connectivity.

A fresh read-only reviewer identified four Important findings. Regression
tests reproduced and verified fixes for stale verification surviving state
changes; open details suppressing same-revision changes; discarded
verification/resolution evidence; and invisible task associations. The
reviewer's transport-health finding was also fixed: successful polling no
longer claims that a disconnected SSE stream is live. No review finding
remains intentionally deferred.

## Decisions made during execution

| Ruling | Reason | Cost if wrong |
|---|---|---|
| Implement locally without waiting for maintainer approval | User explicitly requested implementation; nothing was published | Upstream may require changes or #A fallback |
| Use a container-local isolated worktree | User required all development inside kkt-dev | Review requires using that worktree/branch |
| Consolidate into one verified integration commit | Avoid interim unused code under warnings-as-errors | Coarser commit history |
| Retain external reference rows per home channel | Preserve channel context without cross-channel workflow policy | Repeated PR rows in aggregate views |

## Limits of verification

No manual visual-layout inspection, capacity benchmark, or independent
two-second latency measurement was performed. Replay limits are explicit,
tested safety bounds, not throughput claims. Periodic semantic and forge
verification still requires agents running in the deployment's external
runtime/scheduler. Kkt does not wake agents. Upstream acceptance is unconfirmed.
No issue, push, PR, or merge was performed.
