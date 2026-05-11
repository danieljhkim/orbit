## Context
Planning duels already write the winning plan markdown to `task.plan`, but operators were forced to extract the plan's "Context Files" section and push it to `task.context_files` by hand (see T20260509-7's post-hoc fix). `context_files` is the canonical machine-readable handoff to file-lock, focused-read, and scoped-agent consumers; leaving it empty silently degrades every downstream tool that depends on it.

## Decision
During duel resolution, `writeback_planning_duel_task` parses the normalized winning plan for a "Context Files" section and replaces `task.context_files` with the canonicalized entries when extraction succeeds. Section recognition is deliberately strict to keep the failure mode safe (preserve existing field) rather than best-effort:

- A heading line at level `##` or `###` whose trimmed, case-insensitive text equals `context files` or `context_files` (a single trailing `:` is permitted, additional words are not). The section body extends to the next heading of equal-or-higher level, or to end-of-string.
- Within the section body, unindented `- ` or `* ` bullets contribute one entry each: the first inline-code span on the line, otherwise the first whitespace-bounded token after the marker. Sub-bullets and prose lines are ignored.
- Each entry is canonicalized via `orbit_common::utility::selector::canonical_selector`. Raw paths upgrade to `file:` (or `dir:` if trailing `/`); already-canonical `file:` / `dir:` / `symbol:` selectors round-trip unchanged. Entries that fail canonicalization are dropped and reported as `OrbitEvent::PlanningDuelContextFileSkipped` for observability.
- Duplicates collapse in first-seen order. The replace-not-merge semantics mirror `task.plan`: the winning plan is the new source of truth.

When the section is absent OR recognized but yields zero canonical entries (placeholder / all-unparseable), the writeback leaves `task.context_files` untouched. Both branches are asymmetric-with the right safety bias: clearing a curated field on resolution would silently destroy operator state.

The plumbing adds a single optional field to `TaskAutomationUpdate` (`context_files: Option<Vec<String>>`, default `None` = leave untouched, `Some(v)` = replace). The store layer's `TaskRecordUpdateParams.context_files` already supports this shape, so no store changes are required. Plan-writing flows that aren't duel-mediated are explicitly out of scope for this ADR.

## Consequences
- The duel-resolution writeback is no longer a half-conversion: structured task fields stay in sync with the persisted plan markdown.
- Section-recognition heuristics drift between writers is bounded by the strict rule above; future planner agents that emit non-conforming shapes simply fall back to the preserve-existing branch instead of triggering best-effort guesses.
- A new `TaskAutomationUpdate.context_files` field touches every existing automation call site, but the `..Default::default()` pattern keeps each site at the "leave untouched" default. A regression test in `task_host` guards that contract.
- Operators get a `PlanningDuelContextFileSkipped` event channel for debugging stale or malformed plan markdown, instead of silently-dropped entries.
