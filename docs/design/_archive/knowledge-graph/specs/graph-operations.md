# Spec: Graph Operations (Parked)

**Status:** Parked — no live consumer.
**Last updated:** 2026-05-12

This spec described the sidecar `.orbit/operations/<commit_sha>.json` log format and the eight-op taxonomy (`create`, `delete`, `rename`, `move`, `change_signature`, `change_body`, `split`, `merge`) for Orbit-owned symbol-level write operations. It was drafted under [T20260421-0543] when the read-side consumer was the task-attribution walker that later lived under the knowledge pipeline.

[T20260506-11] / ADR-029 removed graph task attribution after a 10-day audit found 0 uses of the reverse-lookup parameters across 961 graph tool calls. The walker, the `task_ids` field, and the read-side hook were deleted. This spec therefore has no live consumer — the operation log it defined is not read by any current pipeline stage.

## Why This Stays in the Repo

The taxonomy itself may still inform a future graph write tool. ADR-010 captures the eight-op shape and the `stable_id` discussion at the decision level; if a producer ships, the design doc and a fresh consumer story will need to be written together rather than re-using this spec's attribution-shaped framing.

## What to Do Before Reusing This Spec

1. Identify the new consumer. The original consumer was attribution preservation; that motivation is gone. A new consumer (e.g. precise-by-construction renames in a future write surface) would need its own ADR and acceptance criteria.
2. Re-derive the schema in light of that consumer. The envelope, the `stable_id` field, and the validation rules (`producer.name`, tree-diff reconciliation, advisory-authoritative semantics) were all motivated by attribution-walker behavior and should not be ported wholesale.
3. Decide whether the sidecar location (`.orbit/operations/<commit_sha>.json`) is still appropriate or whether a graph-internal location is better.

## Historical Reference

The full original spec is preserved in git history at commit [`aa0950a2`](aa0950a2)'s parent (the `[T20260426-0622]` predecessor commit) and earlier. Run `git log --follow -- docs/design/_archive/knowledge-graph/specs/graph-operations.md` to retrieve it.

## Task References

- **[T20260421-0528]** — Historical `task_ids` schema + git history walker; removed by [T20260506-11].
- **[T20260421-0543]** — This spec, drafted as the producer-side contract for the (now removed) attribution consumer.
- **[T20260506-11]** — Remove graph task attribution; this spec's only consumer was deleted here.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
