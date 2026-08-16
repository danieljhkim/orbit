## Context
Superseded ADRs retain the rejected alternatives and constraints that explain current architecture. Ignoring their bundles made clean clones incomplete and created a recovery deadlock: exact-id restore refused a readable federated copy while guarded worktree GC correctly refused to delete its only copy.

## Decision
Superseded ADR bundles are published decision history and travel with the repository at their original IDs and supersession metadata. Proposed drafts remain unpublished and ignored. Orbit provides an operator reconciliation command that copies a complete byte-identical bundle from an explicitly named registered worktree into the current registered checkout without changing allocation ownership or lifecycle state; it validates source and destination identity, metadata and body completeness, destination absence or byte equivalence, and the allocation snapshot before the atomic publication rename.

## Consequences
- Clean clones retain accepted, superseded, and deleted decision history, including rejected alternatives.
- Rejected alternative: keep superseded bodies local-only and rely on design-doc summaries. This loses the authoritative body and recreates the guarded-GC deadlock.
- Rejected alternative: permit manual file copies or make exact-id restore overwrite a readable federated record. This bypasses validation or fabricates fresh metadata instead of preserving the published bundle.
- Cost: repository history and checkout size grow with every superseded ADR, and reconciliation adds locking and validation complexity to the operator CLI.