## Context
The `orbit design check` and `make check-design-docs` surface compared design-doc `Last updated:` values with referenced code commit dates, but release work showed the signal was brittle and misleading: fresh checkouts made mtime-based fixtures fail, code edits usually did not invalidate prose claims, and the easiest remediation was date-only bumping rather than content review. ORB-00111 would have made the check deterministic by rebasing it on git committer dates, but that still preserved the wrong per-PR signal.

## Decision
Delete the decay-check surface: remove the `orbit design check` CLI subcommand, the `orbit.design.check` MCP tool, the legacy wrapper script, the Make target, and check-only tests. Keep the useful design-doc tooling for `init`, `list`, and `show`, and rely on the same-PR documentation update rule plus code review as the quality gate.

## Consequences
- Design-doc scaffolding and inspection remain first-class while the misleading freshness gate disappears.
- ORB-00111 is superseded because determinizing the old check would not fix the wrong-signal behavior.
- Alternatives considered: content-level structural linting could still be valuable later under a different command, and periodic human audits can target real decay on a slower cadence.
- Cost: Orbit loses an automated stale-doc check, but there is no documented case where it caught a real bug that the same-PR update rule and review would have missed.