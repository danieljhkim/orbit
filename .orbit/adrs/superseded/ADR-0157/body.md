**Context.** Recency and manual priority do not capture whether a learning is still load-bearing. An older learning that agents keep relying on should outrank a newer marginal note, but `updated_at` only moves when the learning body changes. The natural re-validation moment is duplicate-check: an agent reads a candidate learning, decides it already covers the concern, and does not author a competing record.

Alternatives considered:

| Approach | Profile |
|----------|---------|
| **Keep recency + priority only** | No new state. Continues conflating "was once written" with "is still useful." |
| **Global vote count** | Simple. Lets ancient high-volume learnings outrank recently useful ones forever. |
| **Task-anchored decayed votes** | Captures repeated usefulness across work contexts while letting old signal fade. Requires a sidecar file and idempotency policy. |
| **SQLite vote mirror first** | Fast summaries. Adds schema/cache complexity before measured need. |

**Decision.** Each learning may have `.orbit/learnings/<id>/votes.jsonl`, created lazily on first vote. Each row records `learning_id`, `voter_model`, `voted_at`, and `task_id`. V1 rejects votes without `task_id`; idempotency key is `(learning_id, voter_model, task_id)`. Search ranking filters by scope first, then sorts by decay-weighted vote score, `priority`, `updated_at`, and `id`. Default half-life is 180 days; `ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS=0` disables decay for raw-count behavior.

Votes are derived from per-learning JSONL on read. `orbit learning sync` validates vote files but does not rewrite them or mirror them into SQLite.

**Consequences.**
- Load-bearing learnings accrue a ranking signal without mutating the YAML body or bumping `updated_at`.
- Duplicate-check becomes constructive: "this already exists" reinforces the existing record instead of producing a duplicate.
- Per-learning files keep write contention local; same-learning upvotes serialize with a per-learning lock and append atomically.
- Cost: vote spam is possible if agents upvote reflexively. Task anchoring, idempotency, and decay reduce but do not eliminate that risk.
- Cost: search now opens one small votes file per matched learning. This is acceptable for the expected 1-20 row matched sets; a SQLite summary mirror is deferred until measurement shows a need.