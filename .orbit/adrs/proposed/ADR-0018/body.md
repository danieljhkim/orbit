## Context
Two extremes were on the table for ADR content storage: (a) every field structured (`context`, `decision`, `consequences` as separate YAML strings with cost line as its own array entry), enabling queries like *"show every ADR whose Cost mentions latency"*; (b) one big markdown blob with all metadata as filename / index. (a) buys queryability at the cost of write friction; (b) keeps writing easy but defeats the structured-store rationale. A third option — body as a YAML file with named sections (`content.yaml` with `context:` / `decision:` / `consequences:` keys) — was considered and rejected: prose-in-YAML fights multi-line strings, defeats markdown rendering, produces worse `git diff` output, and diverges from `task_store`'s precedent.

## Decision
Hybrid: envelope YAML (`adr.yaml`) carries structured metadata (id, status, owner, related_features, related_tasks, supersession, timestamps); a sibling markdown file (`body.md`) holds the human prose (Context / Decision / Consequences). The split matches `orbit-store::task_store`'s existing pattern (envelope + plan.md + execution-summary.md).

## Consequences

- Metadata queries are fast; body remains comfortable to write and diff.
- The cost-line rule from [CONVENTIONS.md §4](../CONVENTIONS.md) ("every ADR must name at least one cost") becomes a body-parse check rather than a structured-field invariant. The lint runs against `body.md` with a one-line regex (`^- Cost:`).
- Markdown rendering, syntax highlighting, and editor support work without configuration.
- Cost: queries like *"every ADR whose Cost mentions latency"* require FTS5 over the body, not a typed lookup. Acceptable trade-off until corpus size justifies promoting Consequences to structured form. The §1.4 open question in [3_vision.md](./3_vision.md) revisits this.

---
