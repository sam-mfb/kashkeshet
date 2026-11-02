# Local PR draft — optional read-only channel workboard

This draft is local only. Do not publish without explicit authorization.

Agents can opt into ordinary versioned channel reports for tasks, requests,
PRs and issues. The web view projects complete bounded message history into
compact expandable rows, a global pending-request inbox, channel sections and
external identity groups that retain separately attributed channel facts.
It adds only authenticated GET routes; no workboard response writer, tool,
provider fetcher, scheduler or human-approval record is added.

Existing-install behavior is preserved: no migration, MCP contract change or
agent instruction requirement. Existing contract, migration, backup, admin and
web-login checks pass in the 1011-test Rust suite; clippy and fmt pass. Both
jsdom scripts pass. The real Chromium smoke uses an ephemeral loopback server
and scratch DB, and measured five report-to-frame updates below 56 ms locally,
with focus/scroll and mobile-width checks. These are smoke results, not remote
or loaded performance promises.

History budgets remain 10,000 messages / 16 MiB / 1,000 items per channel and
20 selected channels. Exhaustion is explicit. External facts require agents
running in the deployment's existing runtime; kkt does not schedule them or
fetch forge URLs. Reports of outside resolutions are agent claims and never
human authorization. Upstream fit and final independent review are pending.

See the 2026-11-02 workboard audit for individual regressions, validation
commands, implementation rulings and the remaining gates.
