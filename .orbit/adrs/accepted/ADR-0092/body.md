## Context
ADR-011 shipped `ConfigKey` and `Column` extractors that emitted one leaf per top-level YAML/JSON/TOML key and one leaf per CSV/TSV header column. Those leaves had `source: ""` because the body was the file's source rather than a sub-span. Downstream tools special-cased empty-source leaves; search namespaces filled with `name`, `version`, `dependencies`, etc. from every config file in the repo; and the unit agents actually navigate to is the file (`package.json`, `Cargo.toml`, `tsconfig.json`), not the key. The same change also lands before the `benchmarks/graph-latency/` Phase 0 baseline freeze, so v1 numbers measure the leaf model we are keeping.

## Decision
`ConfigExtractor` (YAML/JSON/TOML) and `TableExtractor` (CSV/TSV) return zero leaves. `FileKind::Config(_)` / `FileKind::Table(_)` classification stays so file-level filtering keeps working, and the file node still carries source so `orbit.graph.search` substring queries and `orbit.graph.show` against config selectors return file-level content. Markdown extraction (`Section { depth }`) is explicitly out of scope and unchanged — section bodies have real content and a forward path to per-section embeddings.

## Consequences
- Removes the `source: ""` corner case from downstream tools.
- Cuts leaf count for config-heavy repos (tsconfig matrices, helm charts, monorepo CI manifests) without losing capability — file-level search and `show` continue to work.
- `LeafKind::ConfigKey` and `LeafKind::Column` remain in the enum but are unreachable from extraction; they are not removed because callers may still pattern-match on them and the enum is not `#[non_exhaustive]`.
- Cost: agents that navigate by config key (`name`, `version`, `dependencies`) lose sub-file granularity for those formats and must use file-level selectors plus inspection. There is no migration for old graphs that already persisted ConfigKey/Column leaves; those leaves disappear on the next rebuild.

---

## Task References

Tasks cited by ADRs above:

- **[T20260406-0455-3]** — Add Rust graph extractor.
- **[T20260407-0222]** — Refactor graph storage to content-addressed objects (origin of ADR-001).
- **[T20260409-0550]** — Validate and harden RustGraphExtractor.
- **[T20260409-0656]** — Leaf-level write tool with edit buffering and version chains.
- **[T20260411-0424]** — Consolidate `orbit-knowledge` crate; add tree-sitter extractors, build pipeline, lock store.
- **[T20260416-0236]** (+ `-2`, `-3`, `-4`) — WorkingGraph write-anchor validation, conflict/audit guarantees, canonical selectors, atomic moves.
- **[T20260416-0352]** — Add Go, Java, and JavaScript extraction support.
- **[T20260416-0719]** — Recover from corrupted knowledge graph store.
- **[T20260417-0301-2]** — Harden graph lock store.
- **[T20260417-0302]** — Durable source-file edits in working graph ops.
- **[T20260417-0307]** — Gate and guard graph refresh hot paths.
- **[T20260417-0639]** — Speed up workspace-init graph persistence.
- **[T20260421-0358]** — Branch-scoped refs.
- **[T20260421-0528]** — Historical `task_ids` schema + git history walker; removed by ADR-029 / [T20260506-11].
- **[T20260421-0543]** — Orbit-owned symbol-level write operation schema ([specs/graph-operations.md](./specs/graph-operations.md)).
- **[T20260422-1540]** — Non-code extraction via `FileKind`-dispatched extractors (markdown, YAML/JSON/TOML, CSV/TSV).
- **[T20260423-0452]** — `.orbitignore` scan exclusions and separation from runtime policy access.
- **[T20260426-0139]** — Parallel per-file hashing and leaf extraction with ordered graph merge.
- **[T20260426-0140]** — Changed-path incremental leaf reuse.
- **[T20260426-0141]** — Store-scoped LRU for graph objects and blobs.
- **[T20260426-0220]** — Historical exact task-id filtering through `orbit.graph.search`; removed by ADR-029 / [T20260506-11].
- **[T20260426-0236]** — End-to-end graph build benchmark with scoreboard trend records.
- **[T20260423-0524]** — v3 graph MCP parity sweep (utilization & cost disposition).
- **[T20260423-0607]** — Pointer-only graph reads (deferred; cited by ADR-018 consequences).
- **[T20260426-0402]** — Land v3 retention decision in the ADR index.
- **[T20260426-0453]** — Remove graph write operations from the public tool/MCP surface and standardize on task lock reservations as preflight write guards.
- **[T20260426-0507]** — Historical `orbit graph history` and configurable task-ID regex; attribution behavior removed by ADR-029 / [T20260506-11].
- **[T20260426-2042]** — Move graph CLI behavior behind the `orbit-tools::graph` facade and remove the direct `orbit-knowledge` dependency from `orbit-cli`.
- **[T20260428-1]** — Historical graph task-ID attribution/search alignment; removed by ADR-029 / [T20260506-11].
- **[T20260428-3]** — Expose the full read-only graph tool set through the MCP safe surface for Codex and Gemini.
- **[T20260430-22]** — Compact the knowledge-graph design docs and fold the obsolete evidence log into ADR-018.
- **[T20260505-1]** — Require auto-refresh freshness checks to materialize missing current-branch graph refs before returning fresh.
- **[T20260505-5]** — Bound `orbit.graph.pack` selector gathering and skip inline refresh by default.
- **[T20260505-11]** — Add TypeScript and TSX classification, extraction, and graph search/pack coverage.
- **[T20260505-13]** — Add C# classification, tree-sitter extraction, and graph search coverage for .NET workspaces.
- **[T20260505-14]** — Add Kotlin classification, tree-sitter extraction, and graph search coverage for mixed Java/Kotlin workspaces.
- **[T20260505-15]** — Add Ruby classification, tree-sitter extraction, graph search coverage, and Ruby symbol kinds.
- **[T20260505-16]** — Add C and header classification, tree-sitter extraction, and graph search coverage.
- **[T20260506-11]** — Remove graph task attribution after audited reverse-lookup usage was 0/961; preserve `[T...]` as a local commit-search key.
- **[T20260506-19]** — Normalize knowledge-graph ADR Cost lines and demote folded language instances to coverage records.
- **[T20260509-33]** — Skip symlinked directory entries during scanner traversal and `.orbitignore` discovery.
- **[T20260509-34]** — Use exact git checkout identity instead of commit timestamps for clean graph freshness.
- **[T20260509-64]** — Collapse YAML/JSON/TOML/CSV/TSV extraction to file-as-leaf (ADR-038).
- **[T20260509-65]** — Add `GraphReadOptions` so broad graph reads skip file/leaf source hydration unless a tool opts in.
- **[T20260509-67]** — Bound default-ranking graph search candidate retention with named headroom and hard cap constants.
- **[T20260509-68]** — Replace `overview.top_files` Vec-then-sort with a bounded min-heap top-K.
- **[T20260509-70]** — Build the write-only SQLite secondary index sidecar during graph persistence.
- **[T20260509-71]** — Add the read-side `GraphIndexReader` facade with version check and graceful fallback.
- **[T20260509-72]** — Use the SQLite secondary index for current, unscoped `orbit.graph.overview` summary aggregation.
- **[T20260509-73]** — Wire exact-name and path-prefix graph search through the SQLite sidecar.
- **[T20260509-74]** — Wire `orbit.graph.show` selector resolution through the SQLite unique-selector index.
- **[T20260510-1]** — Restore SQL/fallback equivalence for `orbit.graph.search` (substring on either column).
- **[T20260510-2]** — Restore SQL/fallback equivalence for `orbit.graph.show` `children` (forward leaf pointers).
- **[T20260510-5]** — Extract `orbit_knowledge::commands::*` as the canonical graph command surface and thin graph tools to dispatch/envelope shaping.
- **[T20260510-7]** — Make leaf IDs unique across extractors so SQL fast paths preserve every symbol.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
