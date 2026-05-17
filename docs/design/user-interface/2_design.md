# User Interface — Design

**Status:** Draft
**Owner:** gemini
**Last updated:** 2026-05-16 (ORB-00061)

This document describes the current Orbit UI implementation: the local dashboard assets, the Canon Refined visual rules they rely on, and the telemetry behaviors that must stay consistent with backend data.

## 1. Dense Layout

The dashboard favors wide, dense tables and panels over narrative screens. Tight spacing, small radii, and expandable sunken detail rows preserve hierarchy without hiding root lists. The scoreboard compresses companion metrics into pairs: `tokens` is `total/output`, `tool fail/all` is failed over total tool calls, and `duel w/all` is wins over participated duels. The primary friction column remains reported count only [T20260428-15].

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

## 6. Concerns & Honest Limitations

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

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
