# Proposal: optional channel work view over structured messages

Local issue draft; not published.

## Problem

When agents coordinate in a channel, the operator must read the conversation
to discover queued and active tasks, decisions and approvals awaiting them,
and related PRs or issues. The conversation contains the evidence, but the
current web interface does not provide a consolidated view of that work.

## Proposed boundary

Would an optional web view over a documented structured-message convention
fit kashkeshet's scope?

Agents would publish ordinary channel messages through existing MCP tools.
A versioned, deterministic projection would display those reports as work
items, preserving authors and links to source messages. Unstructured chat
and agents that ignore the convention would keep working as they do today.

The proposal adds no task-specific MCP tools, agent runtime dependency,
forge credentials, listener, or separate database. Messages remain the
source of truth. The existing web login, channel rules, store boundary,
and event notifications remain in use. An in-memory projection cache would
be disposable and rebuildable from history.

## Reviewable increments

1. Document the opt-in convention and add a pure parser/projection with
   fixtures for retries, conflicts, invalid reports, and historical replay.
2. Add a read-only channel Work view with tasks, pending requests, PRs,
   issues, source links, freshness, and automatic refresh/recovery.
3. Add a union of selected channels using the same channel-local model.

Agents would maintain meaning and external facts; the server would only
project and display reports. Periodic verification requires an agent running
under the operator's existing runtime or scheduler. The UI would clearly
mark missing or stale reporting, rather than imply it can wake agents.

## Chosen scope and maintainer feedback

The user selected integration inside kkt, a versioned convention, and a
read-only view. Human responses, operator message writers, and new approval
APIs are deferred. The board shows pending requests; users answer through
an existing agent session or another established venue. Agents can report
external resolution with evidence, clearly labelled as agent reported rather
than authenticated human approval.

The maintainer's acceptance is still unconfirmed. This proposal asks whether
that optional read-only scope fits the project and what convention/support
changes would be needed for an upstream contribution. The user's choices
are settled; they are not evidence of maintainer endorsement.

The intended full design is described in
[Channel workboard — #B](2026-10-30-channel-workboard-design.md).
If the boundary does not fit upstream, the convention and projection can
move to a companion tool instead. No upstream acceptance is assumed.
