---
summary: "User Interface — Decisions"
type: design
title: "User Interface — Decisions"
owner: gemini
last_updated: 2026-08-01
status: Draft
feature: user-interface
doc_role: decisions
tags: ["user-interface"]
---

# User Interface — Decisions

This index records UI ADRs in ascending order. Store-backed entries list their
global ID, title, and status; print their authoritative bodies with `orbit tool
run orbit.adr.show --input '{"id":"ADR-NNNN"}'`. Legacy entries below remain
unchanged until their narratives are separately verified in the ADR store.

Historical note ([ORB-10458]): the entries listed below were authored with local IDs that had no record in the ADR store. They were allocated through `orbit.adr.add`, their narratives migrated into the store verbatim, and their headings rewritten to the allocated global ID. The original local IDs survive as `legacy_ids`, so prior citations still resolve via `orbit tool run orbit.adr.show --input '{"legacy_id":"<feature>/ADR-NNN"}'`. Backfilled here: `user-interface/ADR-00030` → ADR-0284, `user-interface/ADR-001` → ADR-0283.

## ADR-0283 — Canon Refined Aesthetic

**Status:** Proposed · 2026-04 · [T20260427-29] · legacy_id: `user-interface/ADR-001`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0283"}'`.

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

- **ADR-0166 — Grouped Scoreboard Sections** — Accepted.

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

## ADR-0284 — Global, Multi-Workspace Dashboard

**Status:** Accepted · 2026-07 · [ORB-00030] · legacy_id: `user-interface/ADR-00030`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0284"}'`.

## ADR-0256 — Top-Level Nav Is the Operator's Four Tabs

**Status:** Accepted · 2026-07 · [ORB-10444]

**Context.** Top-level nav is the dashboard's scarcest surface, and two of its six entries were not earning a slot: a deprecated review-threads tab that no longer had a backing view, and Scoreboard, a diagnostics-shaped read-only telemetry view sitting beside the operator's actual workflow tabs. Both pushed triage → dispatch → annotate down the visual hierarchy.

**Decision.** The top-level nav is exactly Tasks, Audit, Diagnostics, Knowledge (plus the hash-only `run-detail` route). The deprecated tab is removed outright — nav entry, `TABS` route, pane markup, refresh branch, and CSS — rather than hidden, so no dead asset ships and no route resolves to a missing pane. Scoreboard becomes a Diagnostics subtab routed as `#diagnostics/scoreboard`; its markup moves verbatim into the diagnostics pane so every id `scoreboard.js` renders into (and therefore the `/api/scoreboard` contract and its tests) is untouched. Because the scoreboard needs full width, its subtab swaps the diagnostics two-column layout for a full-width `<main>` while leaving the subtab nav reachable.

Rejected alternative: keep the nav entry and hide it behind a feature flag. Rejected because a hidden tab still ships its assets, its route, and its refresh branch — the cost the removal was meant to recover.

Rejected alternative: promote the subtab nav out of the diagnostics panel header so the scoreboard could replace the whole pane. Rejected as a larger structural change to a shared layout for no operator-visible gain.

**Consequences.**
- The nav reads as the operator's workflow; telemetry lives one level down under Diagnostics.
- Existing `#scoreboard` bookmarks no longer resolve and fall back to Tasks; the view is reachable at `#diagnostics/scoreboard`.
- Cost: the diagnostics pane now owns two `<main>` elements and a visibility toggle keyed on the active subtab.

## ADR-0257 — One-Click Ship and Human-Attributed Comments on Tasks

**Status:** Accepted · 2026-07 · [ORB-10444]

**Context.** The Tasks tab was read-only for the operator's two most common actions. Dispatching a backlog task meant leaving for the CLI or an MCP client, and there was no way to leave a note on a task at all. Both are writes against live state, so the question was how much configuration to expose and whose identity to record.

**Decision.** Ship is one click with no configuration UI: the dashboard posts `{ task_ids: [id] }` to `POST /api/workflows/ship` and nothing else. The crew comes from the task's own record (the pipeline already resolves it) and the mode from the selected workspace's registry binding — so the endpoint's omitted-`mode` default changes from the hard-coded `pr` to that binding's ship mode, falling back to `pr` only when a runtime has no binding. Duplicate dispatch is refused server-side: an explicit task selection whose id is already carried by a non-terminal run is a `409 ship_run_in_flight` naming that run, and the UI additionally holds a per-task guard for the double-click window. Comments post to a new `POST /api/tasks/:id/comments`, which writes through `TaskUpdateParams::comment` into the task's existing review-thread structure — no new field on the task record — and forces a human author: an absent, agent-family, or model-constant author collapses to the `human` label rather than the server process's ambient identity.

Rejected alternative: a UI-only idempotency guard. Rejected because the guard is then lost across a reload and untestable without a JS runner, which the dashboard does not have; the server-side check is deterministic and covers every surface.

Rejected alternative: reuse `PATCH /api/tasks/:id` with a `comment` field for comments. Rejected because that path derives its actor from the runtime's ambient identity, which is exactly the model-constant attribution this decision exists to prevent.

**Consequences.**
- Triage, dispatch, and annotation all complete inside the dashboard; no context switch for routine operation.
- A dashboard comment is always attributable to a person, even when the server runs inside a managed Orbit run.
- Ship failures surface the server's error text instead of silently no-opping.
- Cost: the ship endpoint now scans a bounded window of recent runs before submitting, and a task with a genuinely stuck non-terminal run must have that run cancelled before it can be re-shipped.

**Amendment ([ORB-10544], [ADR-0303]).** "Covers every surface" was true of the intent, not of the placement: the check lived inside the endpoint, so the MCP `orbit.workflow.ship` tool bypassed it. It now lives in the shared ship submission path and every submission surface inherits it. The endpoint's response is unchanged — it projects the shared typed conflict as the same `409 ship_run_in_flight` body.

## Task References

- [T20260427-29] introduced the Canon Refined UI direction.
- [T20260428-13] unified policy-denial sources for the dashboard.
- [T20260428-15] compacted scoreboard ratio columns.
- [T20260430-24] tightened this ADR log without changing decisions.
- [T20260430-29] bounded the live `orbit.log` tail panel.
- [ORB-00144] grouped scoreboard metrics and added knowledge counters plus duel matrix data.
- [ORB-00146] extracted the dashboard and JSON API into the new `orbit-dashboard` internal crate (this document).
- [ORB-00154] unified the Scoreboard tab into a metric-major leaderboard matrix.
- [ORB-00030] made the dashboard global/multi-workspace (workspace-keyed state, `Ws` extractor, serve-from-anywhere, aggregate endpoints).
- [ORB-10444] retired the deprecated tab, folded Scoreboard under Diagnostics, pinned the Knowledge detail pane, and added task ship + comments.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
