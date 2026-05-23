---
summary: "User Interface — Decisions"
type: design
title: "User Interface — Decisions"
owner: gemini
last_updated: 2026-05-18
status: Draft
feature: user-interface
doc_role: decisions
tags: ["user-interface"]
---

# User Interface — Decisions

This append-only ADR log records UI decisions in ascending order. Each entry keeps its status line, cited task ID, decision summary, and at least one explicit cost.

## ADR-001 — Canon Refined Aesthetic

**Status:** Proposed · 2026-04 · [T20260427-29]

**Context.** The dashboard and project website need one visual identity. The prior Trading Terminal direction was dense but too rigid for hierarchical data, review threads, and mixed telemetry.

**Decision.** Adopt Canon Refined: layered dark surfaces, `Inter` plus `JetBrains Mono`, soft semantic colors, compact spacing, and subtle radii.

**Consequences.**
- The UI keeps a serious pro-tool signal while allowing standard web affordances when they improve operator clarity.
- Cost: The design system must be maintained so Canon Refined does not drift into generic dark SaaS styling.

## ADR-002 — Unified Denial Sources for Policy Dashboard

**Status:** Accepted · 2026-04 · [T20260428-13]

**Context.** The Denials 24h tile counted SQLite audit rows and v2 loop denials, but the Policy tab originally scanned only v2 loop JSONL files. Direct CLI denials could increment the tile while the detail table appeared empty.

**Decision.** Aggregate v2 denial envelopes and SQLite `status = denied` audit events in the policy-denials endpoint. SQLite filesystem denials without an activity fsProfile use `workspace-boundary`.

**Consequences.**
- Audit > Policy is a faithful drill-down for Denials 24h, including direct `orbit tool run` policy denials.
- Cost: The endpoint carries a translation layer because SQLite audit rows lack typed denial fields like `profile` and `path`.

## ADR-003 — Compact Scoreboard Ratio Columns

**Status:** Accepted · 2026-04 · [T20260428-15]

**Context.** The scoreboard had separate columns for output tokens, tool calls, duel wins/losses, and friction triage. After failed tool calls became first-class, the split counters made reliability harder to scan.

**Decision.** Render companion metrics as compact pairs: `tokens` is `total/output`, `tool fail/all` is failed over all tool calls, and `duel w/all` is wins over participated duels. Keep only friction reports in the primary table.

**Consequences.**
- The table presents reliability and participation context in fewer columns, while `0/N` tool failures stays meaningful.
- Cost: Friction accepted/rejected counts and raw duel losses require summary JSON or a future detail view.

## ADR-004 — Bounded Live Log Tail

**Status:** Accepted · 2026-04 · [T20260430-29]

**Context.** The Tasks view keeps `orbit.log` visible beside the task list, but the log panel could grow taller than short viewports and push footer controls below the screen.

**Decision.** Keep the Tasks view in a two-column layout and size `#log-panel` to the available viewport. The log row stream owns overflow scrolling, while filters, buffered-count, and follow-tail controls remain inside the bounded panel.

**Consequences.**
- Operators get one clear scroll target for raw log rows while live-tail controls stay visible during short-screen monitoring.
- Cost: The Tasks view trades narrow-screen stacking for denser columns so the live log remains in the first viewport.

## ADR-0166 — Grouped Scoreboard Sections

**Status:** Accepted · 2026-05 · [ORB-00144]

**Context.** The scoreboard started as one compact per-agent table. Adding knowledge-artifact counters and planning-duel matrix data made the flat table mix delivery attribution, review work, operations, knowledge stewardship, and duel outcomes in one scan path. Alternatives were to keep widening the table, add column groups inside the same table, or split the view into focused sections.

**Decision.** Render the dashboard scoreboard as focused sections: Delivery, Review, Knowledge, Operations, Planning Duels, a family-vs-family Duel Matrix, and Attribution Cleanup for non-canonical rows. Keep compact pair cells where they still help local interpretation, but do not treat the whole scoreboard as one primary flat leaderboard.

**Consequences.**
- Operators can inspect one contribution dimension at a time without conflating task creation, planning, implementation, review, tool usage, and knowledge artifacts.
- Non-canonical attribution rows stay visible but no longer compete with canonical agent families in primary sections.
- No single Rust code anchor; this is enforced by dashboard rendering and design review, and workspace-local ADR comments should not be embedded in shipped dashboard assets.
- Cost: Cross-section comparison now requires scanning multiple tables instead of one row, and future metrics must choose an explicit section before being added.

## ADR-0167 — Extract Dashboard + JSON API to orbit-dashboard Crate

**Status:** Accepted · 2026-05 · [ORB-00146]

**Context.** The Orbit web dashboard (HTML/JS + read-only axum JSON API, ~6300 LOC across web/mod.rs and 14 api/* files plus embedded assets) lived inside `orbit-cli`. The only orbit-cli coupling was the `Execute` trait; everything else was external (axum, clap, ...) or `orbit_core::{OrbitRuntime, OrbitError}`. This was the exact shape already used by the sibling `orbit-mcp` internal crate. Keeping it inside CLI forced every CLI edit to rebuild the heavy axum tree and mixed test targets.

**Decision.** Extract to a new `crates/orbit-dashboard/` internal crate (stability = "internal", `[lints] workspace = true`, direct axum/clap/chrono/... + `orbit-core` dep). The crate owns `ServeArgs`, the `pub fn serve(runtime, args)` entrypoint, all api handlers, the three dashboard assets, router construction, shutdown, and browser-opener. `orbit-cli` retains a ≤60-line delegator (`command/web.rs`) that only re-exports the clap `WebSubcommand::Serve(orbit_dashboard::ServeArgs)` and calls `orbit_dashboard::serve`. `audit_middleware` continues to match on the CLI-local `WebSubcommand` (no behavior change to audit names).

Rejected alternative: moving the `Execute` trait (or a shared command-execution abstraction) into `orbit-common` so the dashboard crate could implement it directly. Rejected because `Execute` is a CLI-dispatch detail (clap subcommand wiring, runtime injection), not a domain primitive; polluting `orbit-common` would have been the wrong layering.

**Consequences.**
- `orbit-cli` no longer has a direct `axum` dependency; incremental `cargo check -p orbit-cli` skips the entire dashboard subtree when only command code changes.
- Dashboard assets live next to the Rust that serves them (`assets/dashboard/` inside the crate); `include_str!` paths are now relative and simple.
- The 14 `*_tests.rs` files now compile as part of a dedicated `orbit-dashboard` test target.
- One more workspace member; the existing CI glob in `.github/workflows/ci.yml` picks it up with no per-crate edits.
- Minor duplication of time-parsing, a handful of JSON projection helpers, and a web-only log tail renderer (to avoid a reverse dependency on orbit-cli or colored output). Future centralization of projections can be a follow-up.
- Wire behavior is identical: same routes, same response bodies, same content-types, default port 7878, `--no-open`, `/healthz` body, startup banner, graceful shutdown.
- Cost: one additional crate in the workspace graph and one more place developers look for dashboard code; the projection helpers are now duplicated until a later task extracts a shared `orbit-core` or `orbit-common` projection layer.

## ADR-0168 — Unified Leaderboard Matrix Scoreboard

**Status:** Accepted · 2026-05 · [ORB-00154]

**Context.** The Scoreboard view had grown into six stacked tables that repeated the canonical agents, repeated headers, rendered sparse zeros, and left relative performance as bare integers. Operators needed one glanceable view of which agent leads each metric without scanning 24 repeated rows.

**Decision.** Render canonical metrics as one metric-major Unified Leaderboard Matrix: metric rows grouped by Delivery, Review, Knowledge, Operations, and Planning Duels, with `codex`, `claude`, `gemini`, and `grok` as fixed columns. Non-zero metric cells include inline bars scaled within the row, tied leaders get an explicit `▲` badge, zero values render as an em dash, the Duel Matrix remains compact below, and Attribution Cleanup renders only when non-canonical agents have non-zero signal.

Rejected alternative: agent-major flat wide table. Rejected because roughly twenty metric columns would force horizontal scrolling at the canonical dashboard viewport.

Rejected alternative: per-agent card grid. Rejected because cards preserve the need to scan separate blocks to answer which agent leads a specific metric.

Rejected alternative: pure heatmap matrix. Rejected because color alone hides precise values needed for operator judgment.

**Consequences.**
- The public scoreboard emphasizes per-metric leaders through visual encoding instead of repeated table chrome.
- The canonical four-agent set remains the primary comparison surface, while attribution cleanup stays conditional and secondary.
- No single Rust code anchor; this is enforced by dashboard rendering and design review, and workspace-local ADR comments should not be embedded in shipped dashboard assets.
- Cost: The denser matrix needs careful row-height discipline when new metrics are added.

## Task References

- [T20260427-29] introduced the Canon Refined UI direction.
- [T20260428-13] unified policy-denial sources for the dashboard.
- [T20260428-15] compacted scoreboard ratio columns.
- [T20260430-24] tightened this ADR log without changing decisions.
- [T20260430-29] bounded the live `orbit.log` tail panel.
- [ORB-00144] grouped scoreboard metrics and added knowledge counters plus duel matrix data.
- [ORB-00146] extracted the dashboard and JSON API into the new `orbit-dashboard` internal crate (this document).
- [ORB-00154] unified the Scoreboard tab into a metric-major leaderboard matrix.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
