## Context
Policy and sandboxing semantics were spread across `orbit-policy`, `orbit-exec`, the `PolicyDef` schema in `orbit-common`, the activity dispatcher, and the v2 host. There was no canonical place to record invariants, the `unrestricted` fallback, or the supervision contract.

## Decision
Create `docs/design/policy-sandbox/` as the canonical design folder, with claude as owner. Auditability owns the recording of denials; this folder owns the *semantics* of allow/deny and the *contract* for how spawned processes are supervised.

## Consequences
- Policy and sandboxing decisions now have one ADR log, one glossary, and a feature-owned spec to cite.
- Cost: this folder cross-links into auditability and activity-job, so when those folders change their cross-references must be kept in sync rather than this folder absorbing them.
