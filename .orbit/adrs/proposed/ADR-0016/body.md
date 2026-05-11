## Context
Today's per-feature numbering (`activity-job/ADR-017`, `knowledge-graph/ADR-017`) means the bare string `ADR-017` is ambiguous. Cross-folder reference requires folder qualification, and any cross-feature decision has no natural home for its number. A subtlety surfaced during cross-review: CONVENTIONS §4a rollups fold N source headings into one body. A scalar `legacy_id` cannot represent the alias relationship — migration would either drop folded paths as resolvable IDs or produce body-less artifacts that violate the required body shape.

## Decision
ADR IDs are globally unique (`ADR-NNNN`, zero-padded). Per-feature paths from existing markdown ADRs are preserved on each artifact as `legacy_ids: array<string>` for historical resolution but are not the primary key. **Rollups carry one `legacy_ids` entry per folded source heading plus the rollup's own source path** — folded headings do not become their own artifacts.

## Consequences

- Cross-feature ADRs have one unambiguous ID.
- A bare `[ADR-0042]` citation in any doc resolves without folder context.
- Both rollup-own and folded-heading citations resolve to the same global ID via `orbit.adr.list --legacy-id=...`.
- Migration must allocate fresh IDs and write `legacy_ids` for every existing entry — non-trivial but mechanical.
- Cost: existing references in git history and commit messages (`see ADR-017`) become ambiguous outside their original folder. `orbit.adr.list --legacy-id=activity-job/ADR-017` resolves them, but no plain grep does. Old PRs and code comments don't get rewritten. The array-valued `legacy_ids` is slightly more complex than a scalar field — parsers must handle the N:1 mapping — but the alternative (dropping rollup aliases or producing body-less artifacts) is worse.

---
