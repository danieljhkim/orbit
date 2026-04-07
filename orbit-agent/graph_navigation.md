# Graph Navigation For Agents

This document captures the current design direction for how Orbit agents should consume and navigate the code graph produced by `orbit-agent`.

The goal is not to hand every agent the full repository or raw files by default. Instead, agents should receive a structured, navigable view of the codebase centered on relevant graph nodes and their lineage.

---

## Core Idea

Agents do not need the whole repository as initial context.

What they need is:

- the global graph as an index they can navigate
- a relevant root node or lineage assigned to them
- node-specific context views for `dir`, `file`, and `leaf`

From that starting point, the agent can move outward through the graph only as needed.

---

## Graph Model

The code graph currently has three structural levels:

- `DirNode`
- `FileNode`
- `LeafNode`

### Containment

The tree represents containment only:

- directories contain directories and files
- files contain leaves
- leaves may contain nested leaves

Examples:

- a directory contains subdirectories and source files
- a file contains top-level functions, classes, traits, impls, etc.
- a class leaf may contain method leaves

This graph is intentionally focused on structure first. Other relationships such as imports, calls, inheritance, or references should be added later as edges rather than overloaded into the containment tree.

---

## What Agents Actually Need

Agents should not be forced to read raw graph object files directly for every task.

Instead, each node type should expose its own derived context view.

### Dir Context

A `DirNode` context should provide:

- directory identity and path
- child directories
- child files
- optional subsystem summary
- lock state

This gives an agent a subsystem-level map.

### File Context

A `FileNode` context should provide:

- file identity and path
- imports
- top-level leaves
- optional file summary
- lock state

This gives an agent a concrete edit surface without forcing the full file body into every prompt.

### Leaf Context

A `LeafNode` context should provide:

- node identity
- `name`
- `kind`
- signatures
- line range
- parent file
- child leaves
- current `source`
- change `history`
- lock state

This gives an agent a precise unit of behavior to inspect or modify.

---

## Initial Agent Spawn Model

When an agent is first spawned, it should receive:

1. access to the overall graph
2. the root-level node or lineage relevant to its assignment
3. any lock ownership associated with that lineage

That means the initial prompt/context can stay narrow while still letting the agent navigate when needed.

Example:

- planner selects a relevant `LeafNode`
- worker receives:
  - graph access
  - that `LeafNode`
  - its parent `FileNode`
  - possibly the containing `DirNode`

The worker then expands only when necessary.

---

## Navigation Model

Agents need graph operations, not giant graph dumps.

The runtime should eventually provide navigation primitives such as:

- `get_node(node_id)`
- `get_parent(node_id)`
- `get_children(node_id)`
- `get_siblings(node_id)`
- `get_lineage(node_id)`
- `get_dir_context(dir_id)`
- `get_file_context(file_id)`
- `get_leaf_context(leaf_id)`
- `search_nodes(query, kinds=...)`

This makes the graph usable as a navigable memory structure rather than just a stored artifact.

---

## Planning Model

A planner agent can use the graph as a scope ladder:

- `LeafNode` for precise behavior
- `FileNode` for concrete edit surfaces
- `DirNode` for subsystem coordination

Typical planning flow:

1. identify relevant `LeafNode`s from the task
2. expand to parent `FileNode`s
3. expand to ancestor `DirNode`s only if the task crosses file boundaries
4. assign a lineage root to each worker

This allows planning in terms of real containment boundaries rather than vague module names.

---

## Editing Model

Agents should not treat the graph as the canonical source of truth.

Instead:

- the graph is the editable planning surface
- the branch or worktree is the execution truth
- the regenerated graph is the post-change truth

### Proposed Flow

1. planner identifies a relevant node lineage
2. runtime locks that lineage
3. agent reads the node-specific context
4. agent updates `LeafNode.source` as a structured working copy
5. runtime records the prior state in `LeafNode.history`
6. runtime materializes the updated node back into the real file on a branch/worktree
7. tests run against the actual branch
8. if accepted, the graph is regenerated from the new branch state

This keeps structure, editing, and execution separated cleanly.

---

## Why This Works

This design gives agents:

- narrow initial context
- explicit navigation
- precise edit targets
- better parallelization through lineage locking
- structured history per leaf

It also avoids two common problems:

- giving workers the whole repo when they only need a small region
- treating stale graph state as if it were the actual file system truth

---

## Locking

Lineage locking is important for safe multi-agent work.

Current intended behavior:

- `is_locked = true` means the node itself is under active update
- `lineage_locked = true` means the node is blocked because a descendant or related working region is being updated

Lock helpers should be used by the runtime rather than reimplemented ad hoc by agents.

---

## Open Next Steps

- define concrete `DirContext`, `FileContext`, and `LeafContext` schemas
- add navigation APIs over the stored graph
- define how `LeafNode.source` is materialized back into a full file
- add non-containment graph edges later:
  - imports
  - calls
  - inheritance
  - references
- add validation to detect stale leaf edits before branch application

---

## Summary

The graph should be treated as:

- a structural index for planning
- a navigation surface for agents
- a scoped editing surface for leaf-level work

Agents should spawn with:

- graph access
- a relevant lineage root
- the ability to navigate between `DirNode`, `FileNode`, and `LeafNode` contexts as needed

That gives Orbit a path toward precise, multi-agent code editing without starting every task from raw repository exploration.
