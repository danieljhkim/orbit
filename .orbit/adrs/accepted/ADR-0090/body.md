## Context
The `orbit-tools` graph adapters had accumulated ranking, classification, fast-path, and fallback semantics. That made the tool layer the only consumer able to reproduce canonical `search`, `overview`, `refs`, `show`, and pack behavior, while `orbit-knowledge` exposed lower-level services without a command boundary.

## Decision
Introduce `orbit_knowledge::commands::*` as the canonical typed command surface. Commands take validated typed inputs, own graph-service and SQLite-index selection, and return typed results. `orbit-tools` remains responsible for schema registration, raw JSON argument parsing, boundary/path validation, error-envelope mapping, and final JSON response shaping.

## Consequences
- Non-tool consumers can reuse the same ranked search, overview format selection, reference classification, and fast-path fallback behavior.
- Regression tests for graph semantics live with `orbit-knowledge` rather than tool adapters.
- Tool files shrink to dispatch layers and no longer import graph services or index readers directly.
- Cost: command inputs must carry resolved workspace/knowledge context, so adapters still own the boundary-sensitive path validation that depends on `ToolContext`.

---
