## Context
Embedded activity and job assets are materialized into the global resource catalog, but an additive refresh cannot distinguish a retired bundled file from an operator-authored file by name alone. Filename-only pruning would deactivate stale shipped subsystems, but it could also destroy legitimate local resources on legacy installations.

## Decision
Persist a per-resource-kind managed manifest containing the SHA-256 digest last written for each bundled activity or job. Refresh removes retired files only when their bytes still match that digest, moves locally modified retired managed files into a non-catalog backup area, preserves untracked legacy YAML in place, and emits actionable recovery warnings; the same reconciliation implementation governs both resource kinds.

## Consequences
- A current release can retire assets seeded by an earlier manifest-aware release without leaving them active in catalog construction.
- Existing installations without a manifest migrate safely: exact current bundled bytes gain provenance, while untracked YAML remains active and is named in a manual-recovery warning.
- Locally modified retired assets remain recoverable outside active catalog directories instead of breaking unrelated catalog/list operations.
- Cost: managed manifests and preserved-retirement backups add local state and make the first legacy refresh potentially require operator review before an ambiguous stale file can be removed.