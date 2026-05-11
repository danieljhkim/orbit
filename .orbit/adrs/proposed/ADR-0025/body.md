## Context
The artifact schema preserves CONVENTIONS §4's strict requirements: every ADR must have Context, Decision, Consequences, and at least one labeled `Cost:` line. Cross-review revealed that the existing `activity-job` corpus already violates these in `Accepted` entries — ADR-042 has no Consequences section, and ADR-044, -047, -048 have Consequences without a labeled Cost. Strict migration would either reject these (blocking the entire migration) or force a rushed pre-migration cleanup task that bundles unrelated fixes under deadline pressure.

## Decision
Migration runs in **lenient mode by default**. Entries failing the strict rules are imported with `validation_warnings` recorded on the artifact, and listed in `migration-report.md` for owner follow-up. The strict rules still apply to *new* ADRs created via `orbit.adr.add` after migration; existing entries are grandfathered with a `legacy_validation: warned` flag that the validator treats as a permitted exception until follow-up tasks remediate.

## Consequences

- Migration ships without being blocked by corpus debt accumulated over the past year of activity-job work.
- Owners get a concrete punch list (`migration-report.md`) instead of vague "clean up your ADRs" guidance.
- The strict validator's signal stays clean for new work — strict-mode rejects anything new that lacks a Cost line, so the bar holds going forward.
- Cost: known corpus gaps remain in place until owners file remediation tasks. Nothing automatic forces the cleanup. The store accepts incomplete ADRs in perpetuity if no one acts. Mitigation: `orbit.adr.list --validation=warned` is a one-line query that surfaces the debt, and the [lead-responsibility rule](../../../CLAUDE.md#design-docs) makes it the feature lead's job to clear it.

---

## Task References

- [T20260510-27] — Drafted the adr-artifact design folder as a v2 proposal. The original nine ADRs (001–009) are all `Proposed`; each will be flipped to `Accepted` and cite its shipping task ID as v2 implementation work lands.
- [T20260510-28] — Addressed codex P1/P2 review findings: ADR-002 amended in place to cover `legacy_ids` array and rollup aliasing; ADR-006 amended in place with two named canonical orders; ADR-010 added (search tool placement in `orbit-embed`); ADR-011 added (lenient migration mode as default).

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
