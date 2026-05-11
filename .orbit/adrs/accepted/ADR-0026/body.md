## Context
Auditability is a primary Orbit feature, but its implementation and rationale were spread across README prose, Activity / Job docs, SQLite audit code, loop audit code, and redaction utilities.

## Decision
Create `docs/design/auditability/` as the canonical auditability design folder, owned by codex.

## Consequences
- Audit decisions now have one ADR log and one glossary.
- Cost: auditability overlaps with Activity / Job docs, so cross-links must stay current instead of duplicating the full runtime design.
