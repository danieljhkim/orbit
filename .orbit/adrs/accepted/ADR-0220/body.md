## Context
Orbit cleanup is split between domain commands and startup hooks, while a proposed top-level collector must coordinate global and workspace state without racing live owners. Real alternatives were to keep domain-specific mutation surfaces, place worktree cleanup under `orbit run`, or let each collector define its own locks, reports, and force behavior.

## Decision
Adopt one top-level `orbit gc [TARGET]` family whose default is a non-mutating plan and whose only mutation gate is explicit `--apply`. Every target uses a shared immutable plan/report contract, one host-wide apply lock plus domain-specific atomic revalidation, source-of-truth retention clocks, and non-bypassable containment, symlink, current-owner, and ambiguity protections; `all` composes the same collectors.

## Consequences
- Existing cleanup entry points delegate to the shared collectors or become non-mutating compatibility shims; `orbit run gc` is rejected because retention is storage lifecycle, not workflow execution.
- Global targets take policy only from global config; workspace targets use the existing complete-document workspace-replaces-global rule, with CLI overrides highest.
- Partial failure preserves successful mutations, reports every skip/error, and returns non-zero; reruns are idempotent.
- Audit collection deletes expired envelopes before sweeping blobs and recomputes reachability during blob revalidation; holds, exports, and retained job-run evidence participate in the mark set.
- Code anchors are `execute_gc`, `validate_candidate_path`, and `AuditGcCollector`; the first two enforce the shared lock/manifest/path rules and collector review covers domain invariants.
- Cost: All apply passes serialize behind one host-wide lock and pay per-candidate revalidation, and workspace owners must repeat any global GC settings they want in a complete workspace policy instead of inheriting a merged fragment.