## Context
Two layouts were on the table for on-disk storage: (a) flat files per status — `.orbit/adrs/accepted/ADR-0042.yaml` and `.orbit/adrs/accepted/ADR-0042.md` as siblings; (b) directory per ADR — `.orbit/adrs/accepted/ADR-0042/{adr.yaml, body.md}`. (a) is simpler at small corpus sizes; (b) matches what `task_store` already does and anticipates future per-ADR attachments.

## Decision
Directory per ADR. The layout is `.orbit/adrs/<status>/<id>/{adr.yaml, body.md}`, mirroring `task_store`'s `<status>/<yyyy-mm>/<id>/{task.yaml, plan.md, execution-summary.md, artifacts/}`. ADRs do not date-partition since the corpus is smaller and the ID is already monotonic, but the per-ID directory pattern is identical.

## Consequences

- Consistent with `task_store`. Agents reading both stores reuse the same mental model.
- Per-ADR attachments (diagrams, supplementary specs, review-thread exports, related-decision graphs) live next to the ADR without changing the storage contract.
- Status-directory listing remains efficient at thousands of entries (subdirectories scale better than thousands of sibling files of the same prefix on common filesystems).
- Cost: one extra directory level for every ADR, and `orbit.adr.add` performs an additional `mkdir`. Negligible at expected corpus sizes (low thousands of ADRs even years out), but it does mean a single ADR is no longer a one-line `cat .orbit/adrs/accepted/ADR-0042.yaml` to inspect from a shell — readers go through `orbit.adr.show` or `cat .orbit/adrs/accepted/ADR-0042/adr.yaml`.

---
