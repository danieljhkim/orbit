## Context
V2 runs always constructed both the v2 envelope sink and the loop-level sink. Runs that emitted only envelope events or CLI-backend blobs therefore left zero-byte `.orbit/state/audit/loop/{run_id}.jsonl` files beside populated `v2_loop` files, making the audit tree look noisy and misleading.

## Decision
Keep the loop sink available for HTTP agent-loop events and blob writes, but defer creating `loop/{run_id}.jsonl` until the first `LoopAuditEvent` is emitted. Blob writes continue to use `.orbit/state/audit/blobs/` without creating an empty loop event file.

## Consequences
- Runs with no loop-level provider/tool events no longer leave empty loop JSONL placeholders.
- Cost: consumers must treat a missing loop JSONL file as "no loop events were emitted", not as a missing run; the v2 envelope file remains the canonical run spine.
