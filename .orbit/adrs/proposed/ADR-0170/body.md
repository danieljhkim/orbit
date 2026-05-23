## Context

Orbit accumulates persisted artifacts across two locations: `.orbit/` (tasks, learnings, friction, ADRs, audit DB, indexes, sessions, scoreboards) and `docs/` (design narratives, patterns, runbooks, glossaries). Before [ORB-00163] there was no written rule for which kind of artifact goes where, and the `.orbit/docs/` placement for orbit-docs was actively debated as the obvious-looking alternative.

## Decision

The locating principle is now: **`.orbit/` is for tool-managed artifacts; `docs/` is for human-authored content.** Anything Orbit allocates IDs for, transitions through a lifecycle, indexes, or owns the storage shape of (ADR `adr.yaml`, learning YAML + SQLite index, task files, audit DB) lives under `.orbit/`. Anything authored by humans through PR review, with no Orbit lifecycle (designs, patterns, runbooks, glossaries), lives under `docs/`. Orbit-docs defaults its corpus root to `docs/` and the walker explicitly skips `.orbit/`. ADRs stay under `.orbit/adrs/` because they're tool-managed (allocation IDs, status transitions, supersede chains).

## Consequences

- Discoverability for new contributors: `docs/` is where they read; `.orbit/` is where tools write. Two locations, two roles, no confusion about which one to grep.
- Orbit-docs becomes a thin convention layer over `docs/` — no new on-disk store, no allocation IDs, no lifecycle. Authors keep ownership of layout (recommendation, not enforcement).
- The exclusion is a load-bearing invariant for the walker, not a soft suggestion: [ORB-00163] enforces it with a path-component check (`.orbit` anywhere in the relative path = skipped) and a regression test that points a tempdir root above a `.orbit/adrs/ADR-0001/body.md` and asserts the ADR is not surfaced.
- Cost: ADR corpus stays in a separate query surface from the docs corpus. An agent asking 'what design context exists for feature X' currently has to query both `orbit.adr.*` and `orbit.docs.*`. Whether to fold ADRs into orbit-docs is the v2 design question [ORB-00169] (the task, not this ADR) — but that follow-up exists *because* we chose this strict boundary instead of letting orbit-docs own both corpora.