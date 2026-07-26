## Context

Friction reports once used a dedicated task type, but untriaged reports shared `status: proposed` with human-authored proposals, making scoreboard derivation ambiguous.

## Decision

Add `status: friction` as the creation status for self-reports, infer legacy friction routing at creation, and rebuild `friction_bounty.json` from task history.

## Consequences


- Friction inbox items were separated from human proposals while legacy task records remained readable during migration.
- Cost: legacy untriaged reports need migration, and already-triaged legacy histories depend on existing transition records.

## Provenance

Migrated verbatim from the local heading `auditability/ADR-012` in `docs/design/auditability/4_decisions.md` by [ORB-10458]. Original status line: Superseded · 2026-05 · [T20260510-13]