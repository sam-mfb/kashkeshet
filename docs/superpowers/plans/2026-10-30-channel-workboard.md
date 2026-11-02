# Channel Workboard #B Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking. Delegation requires a separate applicable instruction; none is assumed by this plan.

**Goal:** Add an optional, automatically refreshed kkt web view of agent-reported tasks, decisions, approvals, PRs, and issues, with read-only request details and multi-channel selection.

**Architecture:** #B projects a versioned convention in ordinary channel messages. Kkt retains its existing transport, authentication, database, and notification mechanisms; a pure reducer supplies the work view through rules and the web's `Source` contract. #A remains a fallback, not a parallel implementation.

**Tech Stack:** Existing Rust toolchain, serde/serde_json, SQLite store contracts, Axum, Askama, existing SSE, and Node/jsdom web checks. No new runtime dependency is planned; strict JSON parsing must preserve duplicate-key detection before conversion to `serde_json::Value`.

**Spec:** [Channel workboard — #B](../specs/2026-10-30-channel-workboard-design.md)

## Global constraints

Keep the existing MCP tools and `send` semantics unchanged. Messages remain the source of truth; add no migration, separate service, forge credentials, agent runner, or new listener. All authorization stays in rules, all SQL in `store/sqlite.rs`, and all clock/randomness access uses injected dependencies. The workboard adds GET routes only: no response forms, mutation rules, or operator message writer. The operator can inspect hidden history; all workboard pages are read-only. Base agent instructions do not acquire a mandatory workflow.

**Execution prerequisite:** Read `CLAUDE.md` and the spec, prepare the local maintainer proposal, and obtain direction on upstream scope before substantial product implementation. The user selected #B and requested this plan; that does not establish maintainer agreement. The user has settled integration inside kkt, a versioned convention, and read-only scope. Human response support is deferred beyond this plan. Establish an isolated implementation branch/worktree at execution time. Do not publish an issue or other external message without authorization.

**Commit checks:** Before each implementation commit run `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check`, as the repository requires. The task-level commands below are focused red/green checks in addition to that gate. Do not update MCP contract fixtures: no tool changes are intended.

## Review focus

| Failure condition | Expected behavior and owning task |
|---|---|
| Reports claim that the human approved something | Show agent attribution and evidence; never create authenticated human authorization (Tasks 1 and 3) |
| A successful commit loses its notification | Reconnect, focus, and 30-second reconciliation recover the stored report (Task 3) |
| An item is older than the chat page or rebuild budget | Rebuild full bounded history; explicitly mark unavailable beyond the budget (Task 2) |
| A future observation timestamp or obsolete verification arrives | Neither makes current work appear freshly verified (Tasks 1 and 3) |
| A request is answered outside the board | Only its requester can report resolution with exact revision and evidence; label it as agent reported (Tasks 1 and 4) |

## File map

| Area | Create or modify |
|---|---|
| Convention and projection | Create `docs/workboard.md`, `src/workboard/{mod,parse,reduce,tests}.rs`; modify `src/lib.rs`, `src/config.rs`; create `tests/fixtures/workboard/v1.json` |
| Rules and reads | Create `src/rules/workboard.rs`; modify `src/rules/mod.rs`, `src/rules/tests.rs`, `src/web/{source,real,preview}.rs`; reuse existing `Reads::channel_messages` |
| Web presentation and refresh | Create `src/web/workboard.rs`, `src/web/templates/{workboard,workboard_content,workboard_script}.html`, `tests/web_script/workboard.mjs`; modify `src/web/mod.rs`, `src/web/templates/{base,channel}.html`, `src/web/tests.rs` |
| Adoption and release evidence | Modify `docs/{web,development}.md`, `README.md`, `src/web/preview.rs`, and tests as needed; add no mandatory text to `agent-instructions.md` |

Names below are proposed new interfaces; existing interfaces keep their signatures. New web types stay separate from rules/domain types. Keep `mod workboard` private in `src/lib.rs`.

## Task 1: Freeze v1 convention and build the pure projection

**Deliverable:** A portable parser/reducer and opt-in documentation that produce identical state from full or incremental replay.

**Interfaces:** In `src/workboard/mod.rs`, define owned `MessageInput { id: i64, channel_id: String, author: String, body: String, created_at: String }`, `ProjectionLimits { max_messages: usize, max_bytes: usize, max_items: usize, max_diagnostics: usize }`, `Board`, `Item`, `Diagnostic`, and `ProjectionError`. Expose internally `project(messages: &[MessageInput], now_unix_seconds: i64, limits: ProjectionLimits) -> Result<Board, ProjectionError>` and `Board::apply(&mut self, message: &MessageInput) -> Result<(), ProjectionError>`. `Board` owns items, accepted request IDs, diagnostics, all-message high-watermark, completeness, and agent-reported external resolutions. Final time-dependent presentation is evaluated separately using the supplied `now_unix_seconds`, not persisted in reducer state.

The parser's `parse(body: &str) -> Result<Option<Envelope>, ParseError>` returns `None` for ordinary chat. Use owned, crate-private enums rather than exposing JSON values as the domain API.

**Pin the protocol in `docs/workboard.md`:** Exact marker `kkt-workboard/1\n`; UTF-8 JSON object; reject duplicate/unknown fields, unknown operations, and nesting deeper than 16. Every envelope has `request_id`, `item_id`, `op`. `create` has `kind`, `payload`, and optional `supersedes`; `update` has `base_message_id`, `payload`, and optional `reason`; `withdraw` has base and reason; `verify` has `item_revision`, evidence, optional `observed_at`, and `success`; `resolve` has `base_message_id`, `resolution_note`, `evidence_description`, optional `evidence_message_ids`, and optional `evidence_url`. `respond` and `ack` are unsupported and diagnosed regardless of author. There are no JSON author or channel fields.

| Payload | Exact v1 fields |
|---|---|
| Task | Required `title`, `state`; optional `owner`, `next_step`, `dependency_ids`, `blocker_reason`, `evidence_message_ids`, `responsibility` (`workboard_verifier`), `check_interval_seconds` (positive integer, requires that responsibility and owner) |
| Decision | Required `title`, `question`, `response_mode` (`choice` or `text`), `affected_task_ids`; `options` required for choice, each `{id,label}`; optional `recommendation`, `priority`, `response_url` |
| Approval | Required `title`, `action`, `scope`, `subject`, `affected_task_ids`; optional `expires_at`, `priority`, `response_url`; `subject` is immutable nonempty text containing the exact commit/digest when relevant |
| PR/issue | Required `title`, `provider`, `external_key`, `url`, `task_ids`, `status`; optional `checks`, `reviews`, `observed_at`; strings for facts are escaped, agent-reported text |
| Resolution/verification | Resolution is requester-reported text/evidence, not a typed human answer or approved/rejected outcome. Verification uses `evidence_message_ids` and must reference the current substantive revision to be fresh |

Set fixed initial constants in `config.rs`: 10,000 source messages/channel, 16 MiB cumulative source bodies/channel, 1,000 items/channel, 100 retained diagnostics/channel plus an omitted count, 100 messages/read page, 20 selected channels/request, 16 KiB envelope maximum (existing message-size limit still takes precedence), 128-byte request/item IDs, 200-character titles, 4,000-character other text fields, 100 entries/list, 20 decision options, and 2,048-byte URLs. Wire/document all constants; do not leave fixed limits scattered in handlers. These are plan proposals to validate against the large-channel fixture, not measured capacity claims.

- [x] **1. Write failing fixtures/tests** in `src/workboard/tests.rs`: `ordinary_chat_is_ignored`, `duplicate_keys_are_rejected`, `first_matching_base_wins`, `retry_preserves_original_revision`, `changed_retry_is_diagnostic`, `departed_author_replays`, `verify_does_not_advance_revision`, `future_or_old_revision_verification_is_not_fresh`, `full_and_incremental_replay_match`, `resolve_requires_requester_revision_and_evidence`, and `respond_is_unsupported_even_from_operator`. Pin actual messages 142/143/144: updates 143 and 144 both based on 142 accept 143 only; its identical retry leaves revision 143. Assert malformed reports do not reserve request IDs or prevent later valid messages.
- [x] **2. Run the red check:** `cargo test workboard::tests`. Expect compile failure for the missing module/interfaces, then assertion failures until implemented; distinguish these stages in execution notes.
- [x] **3. Implement the parser, reducer, constants, and fixtures.** Consume complete messages in strictly increasing order for one channel; ordinary chat also advances the high-watermark. Compare duplicate requests as typed payloads, independent of object-key order. Creation fixes kind/creator and validates the creator ID prefix; task/reference changes may come from other authenticated channel authors. Reject missing, self, cyclic, wrong-kind, and cross-channel relationships; dependencies point to already created tasks. Only the requester withdraws/resolves/supersedes pending requests; same-kind replacement applies atomically. State-changing operations advance revision; verification does not. Resolution closes a pending request only with exact base and nonempty note/evidence; keep it visibly agent reported. No envelope creates human authority. Expose all rejection reasons as bounded diagnostics. Define successful exact-revision verification freshness as the earlier of server report time and valid reported observation time; a future reported time is invalid. Derive pending expiry using evaluation time; judge historical external-resolution validity using its stored creation time. Do not trust prose, JSON role fields, or admin-attribution substrings.
- [x] **4. Run green checks:** `cargo test workboard::tests`; expect all new cases passing, including input/relationship limits, invalid dates, safe HTTP(S) URL parsing, unsupported response operations, retry behavior for every operation, and deterministic diagnostics. Fixture format records input messages, evaluation time, expected accepted revisions, and diagnostics so another runtime can implement the convention. Documentation includes a concrete send/read/replay/conflict-retry procedure and explains that stored message acceptance is distinct from projection acceptance.
- [x] **5. Apply commit checks and commit** only the convention/projection deliverable: `feat: define optional workboard message convention`.

## Task 2: Read complete boards through rules and the web contract

**Deliverable:** Authorized, bounded, restart-safe channel projections without a schema change.

**Interfaces:** In `src/rules/workboard.rs`, add `Rules::workboard_view(&self, channel_id: String) -> Result<WorkboardView>` and an internal `project_channel(store: &dyn Reads, channel_id: &str, now_unix_seconds: i64) -> Result<Board>` helper for read-only replay. `WorkboardView` contains channel metadata, hidden/read-only status, projection items/diagnostics, high-watermark, server evaluation time, and availability. Use the existing project `Result` alias and map internal failures to existing `RuleError` codes.

In `src/web/source.rs`, add `fn workboard(&self, channel_id: String) -> BoxFuture<'_, Result<WorkboardView, SourceError>>` with web-owned view types and `fn workboard_policy(&self) -> WorkboardPolicy`. Policy contains `reconcile_ms=30000`, `active_stale_seconds=900`, `external_stale_seconds=120`, and `max_selected_channels=20`, supplied from `config.rs`. Implement both methods in `RulesSource` and `Preview`.

- [x] **1. Write failing rules tests** named `workboard_reads_beyond_chat_page`, `workboard_hidden_history_is_read_only`, `workboard_missing_is_not_found`, `workboard_restart_rebuilds_identically`, and `workboard_history_limit_is_explicit`. Put creation 60 messages before the end with no later update and assert it remains visible; exceed each budget and assert no partial board is returned as complete. Include unrelated malformed/ordinary large messages in byte accounting.
- [x] **2. Run the red check:** `cargo test workboard_`; expect missing interfaces or failing assertions.
- [x] **3. Implement reads and adapters.** Apply `human_readable_channel`/the shared access decision before reading. Read ascending `MessageRange::After(cursor)` pages inside one `Db::read` closure, using existing complete `MessageRecord` bodies, and stop only at end or the explicit budget. The shared helper is storage-only input plus projection logic; its caller owns the access check. Snapshot high-watermark includes ordinary messages. On budget exhaustion return an unavailable board with a specific reason; never synthesize empty success. Map server/domain types to the web's owned types in `web/real.rs`. Preview data includes empty, closed, hidden, conflicting, stale, and over-budget examples. Defer caching entirely; preserve the high-watermark contract for a later measured optimization.
- [x] **4. Run green checks:** `cargo test workboard_` and `cargo test web::tests`; expect passing real/preview contract tests. Add a representative 10,000-message fixture exercised at the allowed boundary and record elapsed time as evidence, without a flaky timing assertion. If this cannot provide usable bounded reads, resolve scope/budget with the maintainer before adding a durable index or changing the no-migration design.
- [x] **5. Apply commit checks and commit:** `feat: project channel work through existing rules`.

## Task 3: Ship the read-only web view and refresh recovery

**Deliverable:** Useful single-channel Work page, authenticated like existing web pages, with live refresh and honest freshness indicators.

**Interfaces:** Register `GET /channels/{id}/work` and `GET /channels/{id}/work/content` through the existing protected router. New handlers live in `src/web/workboard.rs`. The content endpoint returns server-rendered escaped HTML with `data-channel-id`, `data-high-watermark`, `data-evaluated-at`, and availability. Use Askama templates included from the page; no client-side untrusted HTML construction. The page consumes `Source::workboard` and `workboard_policy`; JavaScript receives numeric policy through escaped template attributes.

- [x] **1. Write failing page tests** in `src/web/tests.rs` for inbox ordering, queued/active/completed grouping, evidence links, diagnostics, empty versus completed copy, stale/future facts, safe links, and escaped script payloads. Write `tests/web_script/workboard.mjs` cases `snapshot_event_race`, `old_fetch_cannot_replace_new`, `reconnect_refetches`, `lagged_refetches`, `missed_event_periodic_refetches`, and `refresh_preserves_focus_and_scroll`, using the existing jsdom fake-EventSource pattern. Pin 30,000 ms reconciliation and 900/120-second stale thresholds from rendered policy.
- [x] **2. Run red checks:** `cargo test workboard_` and, against a running preview, `node tests/web_script/workboard.mjs http://127.0.0.1:7499`; expect missing routes/content/behaviors. Install existing locked jsdom dependencies with `npm ci --prefix tests/web_script` only if needed.
- [x] **3. Implement page, content, and refresh behavior.** Add channel/base navigation without changing default chat. Subscribe to `/events` before the initial content fetch; buffer IDs and refetch if they exceed its watermark. Invalidate selected channel content on messages/channel changes, reconnect, `lagged`, and focus. Reconcile every 30 seconds while visible, coalesce requests with one in-flight fetch and a dirty flag, and reject out-of-order content by watermark plus server evaluation time. Abort when navigating away. Keep the focused item/node and scroll anchor when updating; never display answer/approve/reject controls or treat source links as a human chat composer. Show transport connectivity separately from reporting freshness. Update `docs/web.md` to call this increment read-only.
- [x] **4. Run green checks:** `cargo test web::tests`; start `cargo run --example web_preview -- --bind 127.0.0.1:7499 --seed 7 --rate 1 --page-messages 10`; run both `node tests/web_script/workboard.mjs http://127.0.0.1:7499` and existing `node tests/web_script/load_previous.mjs http://127.0.0.1:7499`. Expect all assertions passing. In a browser, verify keyboard/focus and scroll with live bursts and inspect bounded report-to-display latency against the 2-second healthy-connection target. jsdom alone cannot prove layout or latency.
- [x] **5. Apply commit checks and commit:** `feat: show live channel work in the web interface`. This is the first independently useful upstream PR boundary.

## Task 4: Aggregate channels and finish adoption evidence

**Deliverable:** Bookmarkable multi-channel work view and instructions that make agent update obligations and deployment limits explicit.

**Interfaces:** Register `GET /work?channels=<comma-separated-ids>&hidden=0|1` and `GET /work/content` with the same query. In web `Source`, add `fn workboards(&self, channel_ids: Vec<String>, include_hidden: bool) -> BoxFuture<'_, Result<Vec<WorkboardView>, SourceError>>`. Deduplicate/sort IDs, enforce the 20-channel bound, and use existing operator access. Per-channel unavailable states remain explicit; never omit a requested missing channel as if its work were complete. Reuse the channel-view templates and refresh coordinator, tracking a vector of channel watermarks; no global cursor implies that each selected channel is caught up.

- [x] **1. Write failing tests** for identical display names with different channel IDs, two channel-local tasks sharing a PR URL, duplicate selection, too many channels, adding a channel with reports older than every current watermark, hidden selection, per-channel unavailable state, and stale aggregate fetches. Assert unrelated channel names/counts never enter an item's channel-local relationships. Add documentation fixtures for task handoff, superseded approval, evidenced external resolution, a failed forge check retaining last-good facts, and an overdue designated verifier. Assert every workboard route rejects POST and neither real nor preview source adds a response writer.
- [x] **2. Run red checks:** `cargo test workboard_` and `node tests/web_script/workboard.mjs http://127.0.0.1:7499`; expect the aggregate cases to fail before implementation.
- [x] **3. Implement aggregation and adoption docs.** The human union preserves item keys and channel context; provider/external keys identify references while each channel retains its own presentation and associations. Reference facts are updated by agents' existing forge tools, never fetched by kkt. Document 60-second active and 15-minute terminal verification as suggested agent cadences, not server promises. Use Task 1's optional creator-owned verifier task fields to advertise periodic verification; they do not cause kkt to schedule execution. Show no configured verifier versus overdue checks separately. Explain that users answer through an existing agent session or a requester-supplied safe response URL; the board itself accepts no answers. Display external closure as “Reported resolved by <agent>,” never “You approved.” Extend preview scenarios, README navigation, and `docs/development.md` test commands. Include send/read examples usable by two distinct runtimes without requiring vendor hooks.
- [x] **4. Run release checks:** all three repository commit checks, both jsdom scripts against preview, `cargo test --test contract`, `cargo test --test migrations`, `cargo test --test backup`, and `cargo test --test admin`. Expect unchanged tool fixtures, no migration, existing backup restoration retaining messages, and full board reconstruction after restart. Inspect browser behavior with multiple selected channels and pending request details. Record actual evidence and limits; do not claim unmeasured performance or maintainer acceptance.
- [x] **5. Commit and prepare review:** `feat: aggregate channel work and document agent reporting`. Prepare PR text with the final scope, test evidence, absence of intended existing-install changes, remaining history budgets, and the verifier-running prerequisite. Publishing or opening external discussions remains subject to the user's authorization.

## Completion boundary

The implementation is complete when single- and multi-channel read-only views satisfy the spec, agent reporting and external-resolution limitations are visible, and required checks pass. Human answers, response forms, operator writers, and authenticated approval records are outside this plan; no dormant implementation is required for them.

Continued locally on branch `workboard` from v0.8.0 in kkt-dev. The [2026-11-02 audit and validation record](../reports/2026-11-02-channel-workboard.md) supersedes the old completion claims and records remaining review/UI gates. Checked task steps denote implemented and verified deliverables; they do not claim that historical RED runs were reproduced or that upstream accepted the feature. The user's architecture/convention/read-only decisions are settled; maintainer acceptance remains unconfirmed. Native execution is the suggested default because the four tasks share one reducer and read contract.

## Continuation review gate (2026-11-02)

- [x] Port the two existing commits to current v0.8.0, retaining lanes, memory, avatars and server announcement behavior.
- [x] Reproduce and fix round-one findings; verify typed retries, canonical external groups, all verifier coverage, expanded scenarios and URL parsing.
- [x] Apply human UI direction: compact rows, global Needs-you first, padding and readable statuses; retain freshness wording.
- [x] Run Rust/jsdom/real-browser checks and update the agreed local screenshots.
- [ ] Obtain a fresh independent review with no Critical or Important findings and relay the updated UI for human feedback.

Pushes, PR creation and publishing the upstream proposal remain unauthorized.
