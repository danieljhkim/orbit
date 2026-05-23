## Context

Orbit ships three knowledge surfaces for agents (learnings, ADRs, design docs) and is adding a fourth, orbit-docs, as a storage-agnostic indexed-corpus surface for human-authored docs ([ORB-00163]). Without a constrained shape, the corpus drifts into the same per-feature ad-hoc Markdown that existed before, and the retrieval primitive becomes a substring search over arbitrary YAML, which is unrankable.

## Decision

Numbered orbit-docs frontmatter is locked at exactly six fields: `type` (one of `design|pattern|context|glossary|runbook`, required), `summary` (non-empty single line, required), `tags` (string list, optional), `paths` (glob string list, optional), `related_features` (string list, optional), and `related_artifacts` (string list with ID-prefix dispatch — see [ADR-0171], optional). `type` and `summary` are strict; everything else is opportunistic. A tolerant indexer infers missing fields from directory and filename heuristics so legacy docs are discoverable without a forced migration.

## Consequences

- Retrieval-quality lever: ranking has predictable fields to score (`summary` text, `tags` exact, `type` exact). Future semantic ranking ([ORB-00168]) layers on top without renegotiating the schema.
- Indexer can be tolerant: dir-and-filename heuristics infer `type` and `summary` when frontmatter is absent, so the seed corpus works on day one ([ORB-00163] migrated 14 `4_decisions.md`, 12 sibling design docs, and 4 design-pattern docs).
- Cost: the schema is *closed*. Any seventh field (e.g. `last_updated`, `status`, `replaces`) requires another ADR. Plugin authors who want richer metadata must either piggyback on `tags` or argue for a schema extension. We chose closed-by-default over open-bag-of-fields specifically to keep the retrieval surface rankable.