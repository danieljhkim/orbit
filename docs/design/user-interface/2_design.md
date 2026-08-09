---
summary: "User Interface — Design"
type: design
title: "User Interface — Design"
owner: gemini
last_updated: 2026-08-09
status: Draft
feature: user-interface
doc_role: design
tags: ["user-interface"]
---

# User Interface — Design

This document describes the current Orbit UI implementation: the local dashboard assets, the Canon Refined visual rules they rely on, and the telemetry behaviors that must stay consistent with backend data.

## 1. Dense Layout

The dashboard favors wide, dense tables and panels over narrative screens. Tight spacing, small radii, and expandable sunken detail rows preserve hierarchy without hiding root lists. The scoreboard groups per-agent metrics into Delivery, Review, Knowledge, Operations, and Attribution Cleanup sections so task attribution, review work, tool reliability, and knowledge artifacts are scanned separately. Compact pair cells stay local to the sections where they add context: `tool fail/all` is failed over total tool calls [T20260428-15] [ORB-00144] [ADR-0166].

## 2. Layered Palette

The UI uses layered dark surfaces instead of flat black: base canvas, elevated panels, sunken wells, and accent washes. Status color should stay muted and distinct; exact token values live in `./specs/theme.md` and the dashboard CSS.

## 3. Typography

`Inter` carries labels, headings, and prose. `JetBrains Mono` is reserved for IDs, metrics, timestamps, code, and log streams so numeric and diagnostic data stays aligned.

## 4. Live Status

Live processing is visible through pulsing dots, spinners, buffered-log counters, periodically refreshed tiles, and compact ticker-style values. The `orbit.log` panel is viewport-bounded; overflowing rows scroll inside the log stream so footer filters and follow-tail controls remain visible [T20260430-29]. Motion is functional: it points to active work without making the operator read raw logs first.

## 5. Dashboard Telemetry Consistency

Summary tiles and drill-down panels must agree. Audit > Policy is the detail view for the Denials 24h tile, so `/api/diagnostics/denials` combines v2 loop JSONL denial rows with SQLite `status = denied` audit events. SQLite filesystem boundary denials without an activity fsProfile use the stable `workspace-boundary` label [T20260428-13].

Run Detail > Steps now includes compact per-step agent log expanders for CLI-backed activity steps [T20260508-14]. The UI renders bounded stdout and stderr previews from `/api/runs/:id/logs`, distinguishes stderr blocks from stdout blocks, highlights structured `ERROR <target>:` lines, and keeps blob references behind the API so operators do not need to resolve content hashes manually.

Diagnostics has an Errors sub-tab after [T20260508-14]. It renders recent backend error rows independently of Metrics and Policy, combining Orbit process ERROR events with structured agent stderr rows. Rows with `job_run` provenance route back to the owning Run Detail step so error triage stays connected to workflow context.

Diagnostics no longer has a Friction sub-tab after [ORB-00060]. The Friction name is reserved for append-only `.orbit/frictions/` artifacts, while audit-derived negative run signals stay visible in Recent Runs. Recent Runs joins `/api/job-runs` with `/api/diagnostics/friction` client-side by run id (`run_id`/`job_run`) and keeps the table sortable across `denials`, `tool fails`, and `duration`; the duration cell can carry the long-run flag when the diagnostics source supplies one. This preserves column continuity with the existing compact dashboard telemetry direction from [T20260428-15].

Knowledge is now a top-level dashboard tab after [ORB-00061]. Its first sub-tab, Learnings, mirrors the dense task-list pattern: a left scan table backed by `/api/learnings`, a right detail panel backed by the same learning JSON shape as CLI/MCP output, and compact stats tiles for `total`, `superseded`, and `last indexed`. Supersession stays an explicit local action (`POST /api/learnings/:id/supersede`) guarded by the localhost-origin middleware, so curation can happen without leaving the dashboard.

Knowledge detail panels stay pinned while the artifact list scrolls after [ORB-10444]. The list grows well past a viewport, and scrolling it used to carry the pane the operator was reading out of view. The detail panel is sticky below the fixed chrome (header, tabs, health strip) and bounded to the remaining viewport height, so detail content taller than the screen scrolls inside the pane rather than being clipped. The single-column breakpoint unpins it, where the pane already stacks below the list.

## 6. Top-Level Navigation

The top-level nav carries exactly the operator's four workflow surfaces — Tasks, Audit, Diagnostics, Knowledge — plus the hash-only `run-detail` route [ORB-10444] [ADR-0256]. A deprecated review-threads tab was removed outright rather than hidden: nav entry, route, pane, refresh branch, and styles all went, so no dead asset ships and no route resolves to a missing pane. Scoreboard is diagnostics-shaped telemetry rather than a workflow surface, so it moved under Diagnostics as the `#diagnostics/scoreboard` sub-tab. Its markup moved verbatim, so every element `scoreboard.js` renders into — and therefore the `/api/scoreboard` response contract — is unchanged; the sub-tab swaps the diagnostics two-column layout for a full-width one while keeping the sub-tab nav reachable.

## 7. Task Write Actions

The Tasks tab is writable for the two actions that otherwise force a context switch to the CLI [ORB-10444] [ADR-0257].

**Ship** appears on `backlog` tasks and is one click with no configuration UI: it posts only the task id to `POST /api/workflows/ship`. The crew is resolved by the pipeline from the task's own record and the mode from the selected workspace's registry binding, so that endpoint's omitted-`mode` default is the workspace ship mode (falling back to `pr` for a runtime with no binding). The resulting run id and state are surfaced as a notice, and a failed dispatch shows the server's error text instead of silently no-opping. Duplicate dispatch is refused: an explicit task selection whose id is already carried by a non-terminal run answers `409` with code `ship_run_in_flight` naming that run, and the UI holds a per-task guard across the double-click window. That refusal is the shared ship submission path's typed conflict, not an endpoint-local policy, so the MCP ship tool refuses the same duplicate ([ORB-10544], [ADR-0303]).

**Comments** post to `POST /api/tasks/:id/comments`, which writes through the task's existing review-thread structure rather than adding a field to the task record, so a comment survives a reload like any other task history. Authorship is forced to a human identity: an absent, agent-family, or model-constant author collapses to the `human` label, because the dashboard process may itself run inside a managed Orbit run where the runtime's ambient actor is a model.

## 8. Concerns & Honest Limitations

Accessibility still needs a real WCAG pass; responsive behavior remains optimized for wide desktop viewports; raw HTML, CSS variables, and dashboard JavaScript keep the runtime simple but leave duplication across project surfaces.

## Task References

- [T20260427-29] introduced the Canon Refined UI direction.
- [T20260428-13] unified dashboard denial sources for the policy drill-down.
- [T20260428-15] compacted scoreboard ratio columns.
- [T20260430-24] shortened this design doc while preserving current behavior statements.
- [T20260430-29] bounded the live `orbit.log` panel to the viewport.
- [T20260508-14] added Run Detail agent-log previews and Diagnostics > Errors.
- [ORB-00060] collapsed Diagnostics > Friction into Recent Runs diagnostics columns.
- [ORB-00061] added the Knowledge tab and Learnings curation surface.
- [ORB-00144] grouped scoreboard metrics and added knowledge counters.
- [ORB-10444] retired a deprecated tab, folded Scoreboard under Diagnostics, pinned the Knowledge detail pane, and added task ship + comments.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
