## Context
The graph scanner originally filtered only through `git check-ignore`, which meant committed benchmark artifacts and other checked-in generated files still entered the graph and polluted search results. Reusing runtime policy for this would have mixed two different concerns: whether a path should be indexed at all versus whether an activity may read or modify it at runtime.

## Decision
Introduce a scan-only `.orbitignore` layer in `orbit-knowledge`, implemented with the `ignore` crate and evaluated during `scan_repo` before parsing. Keep policy `denyRead` / `denyModify` in `orbit-policy` as a tool-call-time access control surface. Seed the default `.orbitignore` baseline into new workspaces during `orbit workspace init`, but preserve user-edited files once they exist.

## Consequences
- Index quality improves without coupling the scanner to runtime policy semantics or dependencies.
- Users get a visible, editable workspace-root file instead of hidden built-in behavior only.
- Git and Orbit ignore layers compose naturally: `.gitignore` handles Git-owned exclusions, `.orbitignore` handles committed-but-non-indexable paths.
- Cost: there are now two exclusion mechanisms that users can confuse, so the docs have to name the timing and intent boundary explicitly ([2_design.md §2.3]).

---
