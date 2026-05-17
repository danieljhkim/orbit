## Context
Storage choice. Three plausible shapes:

1. **Flat markdown directory.** `docs/learnings/*.md` plus an index file. Easy to author with any text editor. Cheap to grep. Hard to query programmatically (no structured fields), hard to scope (path globs in markdown frontmatter are non-standard), no native lifecycle (supersession, staleness).
2. **Native primitive in `orbit-store`.** YAML on disk + SQLite index, mirroring tasks. Structured fields (`scope`, `evidence`, `status`), atomic mutations via `orbit.learning.*` tools, indexable for sub-10ms lookups. Implementation cost is real but reuses the existing layered store pattern.
3. **Hybrid: markdown bodies + YAML metadata.** Markdown for content, YAML frontmatter for structure. Familiar to many tools. Splits concerns awkwardly when programmatic mutations write to one half and humans edit the other.

The injection layers ([2_design.md §4](./2_design.md)) are the forcing function. Layer 1 has to query "which learnings match this task's context_files" before agent spawn; layer 2 has to do the same per MCP call. Both are hot paths. Grepping markdown frontmatter on every spawn or every tool call is the wrong shape — it makes every layer pay a full filesystem walk for what should be an indexed lookup.

A flat-markdown approach can be retrofitted with an index, but at that point it's a native primitive with extra steps and a less convenient on-disk format.

## Decision
Phase 1 implements `learning` as a first-class Orbit resource: YAML records under `.orbit/learnings/<id>.yaml`, SQLite index under `learnings_index`, MCP/CLI surface mirroring `orbit.task.*`. Tasks were the model because they're the closest existing primitive in shape and lifecycle.

## Consequences
- Hot-path queries are indexed, sub-10ms, and don't pay filesystem-walk cost.
- Lifecycle (`status`, `supersedes`, `superseded_by`) is structurally enforceable.
- The CLI/MCP surface is symmetric with tasks, which lowers the cognitive cost for agents and humans who already know the task model.
- Cost: real implementation work — a new `orbit-store/file/learning_store/` module, a new SQLite table, six MCP tools, six CLI subcommands. This is non-trivial vs. "create a folder and grep it." The bet is that hot-path query performance and lifecycle enforcement justify the build cost over the lifetime of the system.

---
