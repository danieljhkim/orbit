## Context
A task bundle has structurally distinct fields (purpose, summary, plan, acceptance criteria, comments, review threads) of widely varying length. Two embedding strategies exist:

- **Concatenate everything into one document and embed once.** Simplest; one row per task. Loses precision because a strong match in `purpose` is averaged with weak signal from twenty unrelated comments. Long bundles routinely exceed BGE-small's 512-token context, forcing arbitrary truncation.
- **Per-field embeddings, with long fields chunked at paragraph boundaries.** Multiple rows per task. Best-matching field surfaces in the result. Chunking handles the context-window limit cleanly.

The cost of per-field is mostly storage (~5–20× rows per task) and indexing CPU. At BGE-small's 384d, even a generous 20 rows × 10K tasks = 200K rows × 1.5KB = 300MB. Fits comfortably in SQLite, comfortable for brute force at this scale.

## Decision
Phase 1 indexes one row per `(task_id, field, chunk_idx)`. Result formatting collapses multiple field hits on the same task to a single result with the highest-scoring field surfaced as the snippet. Long fields (`plan.md`, `execution-summary.md`) are split at paragraph boundaries with a target of 400 tokens per chunk and 50-token overlap.

## Consequences
- Result snippets point to the actual field that matched, which makes the answer interpretable to users and agents.
- Comments and review messages become independently findable, which directly addresses the "decisions buried in long threads" failure mode in [1_overview.md §1](./1_overview.md).
- Schema's `field` column carries the discriminator without a separate table.
- Cost: 5–20× more rows per task, more storage, more indexing CPU. At phase-1 scale the cost is unproblematic; at much larger scales the per-field strategy may need revisiting alongside the storage upgrade in ADR-002.

---
