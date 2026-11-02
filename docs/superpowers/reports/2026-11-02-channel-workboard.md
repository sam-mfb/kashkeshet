# Workboard v0.8.0 continuation and review evidence

Status: local implementation and round-one fixes verified; a fresh independent
review and final human UI feedback remain pending. Nothing was pushed or
published. Upstream maintainer acceptance is unconfirmed.

## Plan status audit

| Task | State at the inherited implementation | Current local evidence |
|---|---|---|
| 1 — convention/projection | Implemented; raw JSON retry equivalence was incomplete | Typed payload/operation defaults normalize retries for every operation; changed retries remain diagnostics. Portable adoption fixtures execute in the reducer test. |
| 2 — authorized complete reads | Implemented; exact allowed boundary and fresh-Rules rebuild tests were absent | Both tests pass. The 10,000-message boundary test, including insertion and replay, completed in 0.50 seconds; no timing assertion or throughput promise. |
| 3 — read-only channel UI and refresh | Implemented; manual layout/latency evidence was missing | Compact expandable rows, readable status labels, padding without nested card borders, desktop/mobile screenshots, real Chromium report-to-frame, focus and scroll-anchor checks. Freshness wording is unchanged. |
| 4 — aggregate/adoption | Implemented incompletely: canonical external groups, scenario breadth and adoption fixtures were missing | Global Needs-you inbox across selected channels; canonical kind/provider/key groups retain channel-specific facts and associations. Preview includes conflicts, actual history-limit exhaustion, hidden/empty work, supersession, external resolution, failed checks and overdue verification. |

The inherited plan checked boxes described a prior implementation; this audit
and the new evidence are the current record. Historical RED runs were not
reconstructed. Every newly fixed behavior has a regression observed failing
before its implementation. Local continuation was explicitly authorized;
publication remains outside this work.

## Independent review round one

Initial reviewed head: e861ca9; base: origin/main 18ad65b.
Reviewer verdict: 0 Critical, 4 Important, 1 Minor.

| Finding | Fix and reproducing evidence |
|---|---|
| Equivalent omitted/null/default payloads changed retry identity | Canonicalize typed payloads and operations, including optional fields/default lists. `equivalent_typed_retries_preserve_every_operations_first_acceptance`: RED → GREEN, all five operations. |
| External identity was lost; the same PR was rendered independently | Preserve normalized provider/key and kind; present one external group with every selected channel's separately attributed facts. Global pending requests sort by priority, blocked-task count and age. `workboard_aggregates_requests_and_groups_external_identity_without_losing_facts`: RED → GREEN. |
| Later current verifier hid an earlier overdue verifier | Retain every active verifier in the coverage summary. `workboard_keeps_every_active_verifiers_coverage_visible`: RED → GREEN with two different cadences and an overdue assignment. |
| Missing preview/adoption/aggregate/latency evidence | `workboard_preview_exercises_conflicts_budgets_resolution_and_verification`: RED → GREEN; executable adoption fixtures; mixed-watermark and old-channel selection jsdom cases; actual browser smoke described below. |
| Minor: malformed HTTP host/port accepted by prefix check | Use `url::Url` internally; malformed host and port regression RED → GREEN. `url` was already a resolved transitive dependency, now direct; no new resolved package. |

Real-browser checking also exposed focused summaries retaining old verification
text. A jsdom regression reproduced it; updating children while preserving the
focused node fixes both freshness and focus identity. The full Rust suite caught
a preview history-order regression: the archived report now precedes the
operator's hide notice, preserving the prior preview contract.

## Verification

All repository work ran in kkt-dev, image roanutil/dev-container:0.5.0.

| Command | Result |
|---|---|
| `cargo test` | 1011 passed, 0 failed, across 28 result targets; includes existing contract, migrations, backup, admin and web-login suites. |
| `cargo clippy --all-targets -- -D warnings` | Exit 0. |
| `cargo fmt --check` | Exit 0. |
| `node tests/web_script/workboard.mjs http://127.0.0.1:38271` | Exit 0; snapshot races, recovery, history, expiry, focus, navigation, mixed aggregate watermarks and older-channel selection. |
| `node tests/web_script/load_previous.mjs http://127.0.0.1:38271` | Exit 0; four older-page loads and stale-page rejection. |
| `KKT_PLAYWRIGHT_MODULE=target/k1-browser/node_modules/playwright/index.mjs node tests/web_script/workboard_browser.mjs` | Exit 0; Playwright 1.64.1 / Chromium, genuine MCP reports, SSE invalidation, server snapshots and a browser animation frame. |

The browser runner creates its own loopback server on port 0, empty config and
scratch database under target/k1-validation. It never addresses the live server.
It records browser-latency.json and stops its own server on exit. Build the local
binary first with `cargo build --bin kkt`. Playwright and Chromium are optional
manual-validation tools installed outside the committed dependency manifests.
The independent preview at port 38271 contains simulated data only.

Five report-request-start to displayed-frame samples (milliseconds): 29.97, 44.77, 55.59, 44.56, 55.48.
Maximum 55.59 ms against the 2000 ms healthy-local-connection target. This is
local smoke evidence, not a latency bound under load or over a remote network.
Focus survived same-revision verification; a 16-report burst and a new global
request preserved the focused row's DOM identity and scroll anchor. The 390px
viewport had no horizontal overflow.

Screenshots: channel-work.png, request-details.png, aggregate-work.png and
channel-work-mobile.png in the previously agreed local screenshot directory.
They show the same PR with conflicting channel facts, the global request inbox,
an overdue verifier, a history-limit failure, and the compact expandable rows.
Screenshot locations are not published to the remote job board.

## Rulings and remaining boundary

- Existing completed commits were ported rather than rewriting their historical
  TDD sequence. Cost: old RED claims rely on the prior implementation record.
- Global groups require a consistent vector of snapshots. Reject a mixed stale
  aggregate as a whole and retry through normal invalidation/reconciliation;
  never regress one selected channel. Cost: another channel's newer report can
  wait for the next consistent fetch.
- Canonical identity is kind plus trimmed provider/key, with provider casing
  normalized. Do not guess forge-specific key casing, merge conflicting facts,
  or infer cross-channel dependencies. Cost: agents must report consistent keys.
- The URL parser is a direct reference to an existing transitive crate. Cost:
  a direct manifest dependency, without another resolved package or fetcher.

Pending: a fresh independent review of the fixes and human feedback on the
updated screenshots. No claim of a clean final review, upstream acceptance,
production forge accuracy, or a running external verifier is made. Kkt does not
schedule agents or authenticate human approvals through workboard messages.

## K2 — approved presentation polish

Four human-approved refinements are implemented locally, pending a fresh RK2
independent review. Status and type/owner/channel metadata use consistent grid
columns, switching to aligned rows at narrow widths. External groups omit the
repeated attribution boilerplate; the per-row channel metadata and existing
agent-claim notices remain. Verification labels use relative age, with the exact
UTC timestamp available in a title tooltip. Stale / Not verified copy is
unchanged. A history-limit board now uses the existing amber warning treatment,
explains that reported work may be older than the replay budget, links to the
channel history, and suggests asking the agent to report current items in a new
channel. This is a recoverable projection limit, so a warning better fits than
the red error treatment retained for other failures.

Regression evidence: three Rust regressions were observed RED before the
template/mapper changes, then GREEN. Chromium reproduced the ragged status
column with a 142.75px difference before the CSS change. The new
tests/web_script/workboard_polish.mjs measures status and context positions,
checks overflow, and refreshes the four agreed screenshots. Latest two runs
showed 0px spread across nine visible rows at both 1440px and 390px and no
overflowing elements. One earlier run reported mobile overflow without element
diagnostics; subsequent diagnostic runs did not reproduce it. This remains
local browser evidence, not a guarantee across fonts/browsers.

Fresh required checks: cargo test passed 1,013 tests with zero failures across
28 result targets; cargo clippy --all-targets -- -D warnings and
cargo fmt --check exited 0. Both existing jsdom scripts passed.
The genuine MCP/SSE browser smoke passed with samples 32.31, 44.83, 57.98,
43.92, and 52.42 ms; focus, scroll anchoring, and mobile overflow checks passed.
Logs are under target/k1-validation/k2-*; screenshots updated in the agreed
container and host review locations. No existing-install schema, tool contract,
permissions, read-only boundary, replay policy, freshness policy, or retry
semantics changed. Nothing was pushed or published.

### RK2 round one and bounded header fix

RK2 reviewed 351fcc942dd64ba2fc075043733f75edeb19f6c5: zero Critical,
zero Important, one Minor; all four approved K2 refinements accepted.
The Minor finding identified the earlier intermittent overflow: the shared
header did not wrap when the transport label became connecting or disconnected.
The parent authorized this bounded correction.

The browser acceptance now forces live, connecting, and disconnected labels at
1440px and 390px. Before changing CSS it failed at 390px with a connecting
header width of 404px. One media rule now allows the header to wrap at widths
up to 560px. Afterward all three labels report a 390px document width on the
390px viewport; no overflowing elements remain. Desktop document width is
1440px and header height stays 43.5px for all labels. The nine work rows retain
0px spread for both status and metadata columns at both widths.

Required checks passed again: 1,013 Rust tests, clippy with warnings denied,
fmt check, both jsdom scripts, the layout acceptance and the genuine MCP/SSE
Chromium smoke. Latest smoke maximum was 59.70ms; focus and scroll anchoring
passed. Fresh evidence is in target/k1-validation/k2-header-*.log and
k2-layout.json. Four screenshots updated. Deferred freshness copy is unchanged.
A fresh RK2 round-two review remains pending; nothing was published.
