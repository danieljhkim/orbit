# Glossary: Knowledge Graph

Orbit-specific terms used across the knowledge-graph docs and `orbit-knowledge`. Terms with standard industry meanings (blob, content-addressed, index, manifest, hunk, tree-sitter, etc.) are not repeated here — only vocabulary where the Orbit shape is non-obvious. Cross-references point at [1_overview.md](../1_overview.md), [2_design.md](../2_design.md), and [specs/refs.md](../specs/refs.md).

| Term | Meaning |
|------|---------|
| **`.orbitignore`** | Workspace-local, gitignore-compatible exclusion file consumed by the knowledge-graph scan stage. It controls graph inclusion at parse time and is distinct from runtime policy deny rules. See [2_design.md §2.4]. |
| **Attribution (removed)** | Historical pipeline stage that parsed task IDs from commit messages and attached them to nodes. Removed in [T20260506-11]; see [2_design.md §2.2]. |
| **CodebaseGraphV1** | Top-level serialized graph shape: `{ root_dir_id, dirs, files, leaves }`. The `V1` is load-bearing — it pins the on-disk schema. |
| **Command surface** | Canonical `orbit_knowledge::commands::*` entry point for graph operations. Tools parse transport envelopes and shape JSON; commands own query semantics and lower-level service/index selection ([T20260510-5]). |
| **DirNode / FileNode / LeafNode** | The three Orbit node types. `LeafNode` is the in-code name for what the tool surface calls a "symbol" (renamed under [T20260411-0424]; the type name predates the rename). |
| **Fingerprint (dirty)** | Orbit-specific encoding of worktree dirtiness: `sha256(git status --porcelain)` + path count + newest mtime of any dirty file. Drives the dirty-refresh debounce; a stable fingerprint reuses the cached graph. |
| **GraphContextService** | Lower-level read primitive. Holds a loaded graph plus a location index; command modules call it when a query cannot be answered from a sidecar index. Writes do not go through it. |
| **GraphNavigator** | Low-level traversal primitive wrapped by `GraphContextService`. |
| **Identity key** | Cross-build stable key distinct from `id` and `object_hash`. `id` is stable per-snapshot; `object_hash` changes on any field edit; `identity_key` is what the working graph uses to track a node's lineage across rebuilds. |
| **is_locked / lineage_locked** | Two lock flags on every node. `is_locked` blocks body mutations; `lineage_locked` blocks identity changes (rename, re-identification). Both survive rebuilds because they live on the node body. |
| **Leaf** | Internal vocabulary for an extracted symbol. "Symbol" is the public tool surface term ([T20260411-0424]); "leaf" persists in the Rust types. |
| **LeafKind** | Extracted-node kind. Code variants include `function`, `method`, `class`, `struct`, `interface`, `trait`, `impl`, `field`, and `module`; the only emitted non-code variant is markdown `Section { depth }`. The `ConfigKey` and `Column` variants remain in the enum for forward compatibility but are no longer produced after [T20260509-64] (see [4_decisions.md ADR-038]). |
| **Location** | Orbit's selector-friendly path format. Dirs end with `/`; files use the repo-relative path; leaves use `<file>:<qualified_name>`. |
| **Pack** | Orbit-specific render: `pack_json` produces a token-budgeted bundle of selected nodes shaped for prompt consumption. Not a generic archive — the field projection is deliberately agent-friendly. |
| **Ref fallback** | Orbit read-side rule: if `refs/heads/<current-branch>.json` does not exist, fall back to `refs/heads/<default>.json` and emit a stderr warning. Writes never fall back ([T20260421-0358]). |
| **Refresh lock** | `flock` on `.orbit/knowledge/refresh.lock` single-flighting rebuilds across processes. Concurrent callers wait or reuse the in-flight result rather than racing. |
| **RefName** | Validated branch-name newtype. Exists so ref paths can't be constructed from arbitrary strings. |
| **Selector** | Universal addressing primitive for every Orbit graph tool input. Accepts a location, `location:kind` (to disambiguate struct-vs-impl at the same qualified name), or a raw node id. |
| **Structural conflict (removed)** | Historical attribution flag set when both sides of a merge commit touched a leaf. Removed with graph task attribution in [T20260506-11]. |
| **task_ids (removed)** | Historical node field containing Orbit task IDs attributed by the history walker. Removed in [T20260506-11]; commit tags remain local `git log --grep` search keys. |
| **TaskGraphScope** | Scope selector for per-task graph operations — workspace-only, global, etc. Mirrors the Orbit-wide scoping rules. |
| **Working graph** | In-memory overlay on a branch snapshot used during an activity to stage edits without perturbing the persisted store. Persists at activity boundaries only. |
