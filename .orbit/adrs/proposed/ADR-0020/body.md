## Context
Once ADRs are first-class artifacts, the hand-maintained per-feature `4_decisions.md` file becomes redundant — the same data lives in the store. Two ways to handle it: delete the file entirely (readers query the store), or replace its contents with a generated index built from `orbit.adr.list --feature=<name>`. Deletion is simpler but removes the affordance of "open one file to see every decision for this feature." A subtlety surfaced during cross-review: CONVENTIONS expects ascending ADR order in `4_decisions.md`, but real corpora are not strictly ascending (`activity-job/4_decisions.md` already has ADR-048 before ADR-047). The generator needs a named stable sort, not just "ascending."

## Decision
Auto-generate. `4_decisions.md` becomes a build artifact, regenerated from the store via `orbit.adr.list --feature=<name> --format=md` (or an equivalent `make` target). The generated file is committed to git so readers without Orbit installed — and reviewers using only a web git host — can still browse it.

**Two named canonical orders apply:**

| Surface | Order |
|---------|-------|
| Generated `4_decisions.md` (per-feature) | Ascending by **legacy feature ADR number** when present (preserves the historical reading order, including pre-existing non-monotonic cases like ADR-048-before-ADR-047), then by **global `ADR-NNNN`** for legacy-less entries (new ADRs added post-migration). |
| Generated `cross-cutting/4_decisions.md` | Ascending by global `ADR-NNNN`. No legacy ordering applies — cross-cutting ADRs are born under the new scheme. |
| `orbit.adr.list` CLI default | Descending by global `ADR-NNNN` (most recent first; standard browsing order). |

## Consequences

- Browsable affordance is preserved; the store remains the source of truth.
- Generators handle ordering, filtering, and supersession links uniformly across features.
- Per-feature `4_decisions.md` preserves the *exact* reading order operators are used to — including historical quirks — so migration doesn't shuffle the file under reviewers.
- `4_decisions.md` becomes off-limits for hand-editing; CONVENTIONS.md is updated to mark it as generated and the migration task adds a CI check that fails on hand-edits.
- Cost: a generated file in git means every ADR add/update produces diff noise. Generation must be **idempotent** — repeated runs against the same store state must produce byte-identical output, or every commit produces spurious churn. The migration tool implements canonical ordering and stable timestamp formatting from day one. Two named orders also means the generator carries two sort implementations and tests for both.

---
