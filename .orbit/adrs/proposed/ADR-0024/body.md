## Context
[2_design.md §4.6](./2_design.md) routes `orbit.adr.search` through `orbit-embed::vector::VectorStore`. The initial design placed all `orbit.adr.*` tools in `orbit-tools`. Codex flagged the contradiction: `orbit-tools` does not depend on `orbit-embed` (CLAUDE.md architecture diagram), and adding that edge widens the dep graph for one tool. `orbit-embed` already exposes its own `commands::*` surface (`install`, `uninstall`, `reindex`, `stats`) for embedding-adjacent operations.

## Decision
Split the placement. `orbit.adr.{add, show, list, update, supersede}` and the review-thread tools live in `orbit-tools` (no `orbit-embed` dep needed). `orbit.adr.search` lives in `orbit-embed::commands` alongside the existing embedding-related commands, and is registered into the central tool registry from there. `orbit-tools` stays at its current dep set: `orbit-common`, `orbit-exec`, `orbit-knowledge`, `orbit-policy`.

## Consequences

- Crate architecture stays intact. No new edges in the dep graph.
- Search-specific code lives next to the rest of `orbit-embed`'s embedding surface, where the maintainers of `orbit-embed` already operate.
- The tool registry already supports multi-crate registration (tools from `orbit-tools` and `orbit-knowledge` already register through the same surface), so no new infrastructure.
- Cost: `orbit.adr.*` tools are split across two crates instead of co-located. A reader looking for "where is `orbit.adr.add` implemented?" finds it in `orbit-tools`; "where is `orbit.adr.search`?" finds it in `orbit-embed`. The split is principled (dep graph) but does require a doc-comment pointer in each crate so the relationship isn't surprising. Alternative — adding `orbit-embed` as an `orbit-tools` dependency — was rejected because it widens the dep graph permanently for one tool's worth of work.

---
