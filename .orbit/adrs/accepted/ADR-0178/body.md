## Context
Phase 2 left `--tag` and `--path` as no-op filters for ADRs because ADR envelopes had no free-form labels or applicability paths. Reusing `related_features` would collapse constrained feature-folder references into loose tags, and it still would not answer pre-edit path applicability queries.

## Decision
Add `tags: [string]` and `paths: [string]` to ADR envelope YAML, bump newly written envelopes to `schema_version: 2`, and keep v1 readers compatible by treating missing fields as empty lists. `orbit.adr.list` and `orbit search` filter ADRs through these fields with case-insensitive tag equality and glob-containment path semantics.

## Consequences
- Cross-artifact label and path queries can include ADRs alongside tasks, docs, and learnings.
- Existing ADR envelopes are backfilled with explicit empty defaults; owners populate meaningful tags and paths when they next touch a decision.
- No automatic inference from title, body, or related_features is attempted, keeping false labels out of durable decision metadata.
- No single code anchor owns this behavior; it is enforced across the ADR store schema, tool schemas, search command, and their focused tests.
- Cost: The schema bump touches every ADR envelope and adds two author-maintained metadata axes that can drift if reviewers do not keep them current.