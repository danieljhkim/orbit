## Context
The scanner used `Path::is_dir()` while walking files and discovering nested `.orbitignore` files. That follows directory symlinks, so a repository symlink could index files outside the workspace or recurse through cyclic self/parent links. A more permissive option would canonicalize symlink targets, follow only those still inside the workspace, and maintain a visited-directory set.

## Decision
Treat symlink traversal as opt-out by omission: classify entries with `DirEntry::file_type()` and recurse only into non-symlink directories. Apply the same rule to `.orbitignore` discovery. Regular files and non-symlink directories continue through the existing `.gitignore` / `.orbitignore` inclusion pipeline.

## Consequences
- Repository symlinks cannot pull outside-workspace files into the graph by default.
- Cyclic symlinked directories cannot make scan recursion unbounded.
- `.orbitignore` discovery and source-file scanning now share the same symlink boundary.
- Cost: legitimate source exposed only through symlinked directories is not indexed until Orbit grows an explicit, canonicalized, cycle-safe follow policy.

---
