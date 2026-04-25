# Glossary: Knowledge Graph

Orbit-specific terms used across the knowledge-graph docs and `orbit-knowledge`. Terms with standard industry meanings (blob, content-addressed, index, manifest, hunk, tree-sitter, etc.) are not repeated here — only vocabulary where the Orbit shape is non-obvious. Cross-references point at [1_overview.md](../1_overview.md), [2_design.md](../2_design.md), and [specs/refs.md](../specs/refs.md).

| Term | Meaning |
|------|---------|
| **`.orbitignore`** | Workspace-local, gitignore-compatible exclusion file consumed by the knowledge-graph scan stage. It controls graph inclusion at parse time and is distinct from runtime policy deny rules. See [2_design.md §2.3]. |
| **Attribution** | Orbit-specific pipeline stage that parses `\[T\d{8}-\d{4}(?:-\d+)?\]` task IDs from commit messages, maps hunks to leaves by line-range overlap, and unions the IDs onto touched nodes. See [2_design.md §2.2]. |
| **CodebaseGraphV1** | Top-level serialized graph shape: `{ root_dir_id, dirs, files, leaves }`. The `V1` is load-bearing — it pins the on-disk schema. |
| **DirNode / FileNode / LeafNode** | The three Orbit node types. `LeafNode` is the in-code name for what the tool surface calls a "symbol" (renamed under [T20260411-0424]; the type name predates the rename). |
| **Fingerprint (dirty)** | Orbit-specific encoding of worktree dirtiness: `sha256(git status --porcelain)` + path count + newest mtime of any dirty file. Drives the dirty-refresh debounce; a stable fingerprint reuses the cached graph. |
| **GraphContextService** | Read-side entry point. Holds a loaded graph plus a location index and serves every `orbit.graph.*` query. Writes do not go through it. |
| **GraphNavigator** | Low-level traversal primitive wrapped by `GraphContextService`. |
| **Identity key** | Cross-build stable key distinct from `id` and `object_hash`. `id` is stable per-snapshot; `object_hash` changes on any field edit; `identity_key` is what the working graph uses to track a node's lineage across rebuilds. |
| **is_locked / lineage_locked** | Two lock flags on every node. `is_locked` blocks body mutations; `lineage_locked` blocks identity changes (rename, re-identification). Both survive rebuilds because they live on the node body. |
| **Leaf** | Internal vocabulary for an extracted symbol. "Symbol" is the public tool surface term ([T20260411-0424]); "leaf" persists in the Rust types. |
| **LeafKind** | Fixed enum: `function`, `method`, `class`, `struct`, `interface`, `trait`, `impl`, `field`, `module`. Directly inherited from ctags' tag kinds. |
| **Location** | Orbit's selector-friendly path format. Dirs end with `/`; files use the repo-relative path; leaves use `<file>:<qualified_name>`. |
| **Pack** | Orbit-specific render: `pack_json` produces a token-budgeted bundle of selected nodes shaped for prompt consumption. Not a generic archive — the field projection is deliberately agent-friendly. |
| **Ref fallback** | Orbit read-side rule: if `refs/heads/<current-branch>.json` does not exist, fall back to `refs/heads/<default>.json` and emit a stderr warning. Writes never fall back ([T20260421-0358]). |
| **Refresh lock** | `flock` on `.orbit/knowledge/refresh.lock` single-flighting rebuilds across processes. Concurrent callers wait or reuse the in-flight result rather than racing. |
| **RefName** | Validated branch-name newtype. Exists so ref paths can't be constructed from arbitrary strings. |
| **Selector** | Universal addressing primitive for every Orbit graph tool input. Accepts a location, `location:kind` (to disambiguate struct-vs-impl at the same qualified name), or a raw node id. |
| **Structural conflict** | Flag on a leaf set when both sides of a merge commit touched it. Informational — git already resolved the textual conflict; the flag tells the scoreboard the symbol was contested. |
| **task_ids** | Sorted, deduplicated list of Orbit task IDs attributed to a node by the history walker. Brackets stripped in storage. Introduced in [T20260421-0528]. |
| **TaskGraphScope** | Scope selector for per-task graph operations — workspace-only, global, etc. Mirrors the Orbit-wide scoping rules. |
| **Working graph** | In-memory overlay on a branch snapshot used during an activity to stage edits without perturbing the persisted store. Persists at activity boundaries only. |
