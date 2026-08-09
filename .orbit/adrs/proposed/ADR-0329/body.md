## Context
Bubblewrap can only re-bind an exception beneath a read-only parent when the exception anchor exists. The prior hardcoded path/type inventory drifted from effective policy, while preparing every apparent re-allow would materialize paths shadowed by later workspace denies and filename-shape inference could not distinguish dotted directories from extensionless files.

## Decision
Derive Linux write-grant candidates at every provider spawn from the same ordered ResolvedFsProfile used to compile Bubblewrap argv. Materialize only narrow re-allows whose anchor is writable under the final last-match-wins decision; exact rules denote file anchors and `<root>/**` rules denote directory anchors. Confine creation to the canonical managed worktree, reject symlinks in every worktree-owned component and canonical escapes, and translate child-reported EROFS paths into Orbit policy diagnostics after failed invocations.

## Consequences
- Policy evaluation, anchor preparation, mount compilation, and denial diagnostics share one effective rule sequence without a hardcoded path inventory.
- Later workspace denies prevent materialization, while narrower denies below a writable subtree preserve the remaining grant.
- Cost: policy authors must use exact syntax for file anchors and `<root>/**` syntax for directory anchors; an existing target whose filesystem type contradicts that syntax fails closed.
- Cost: post-invocation attribution depends on the child including the attempted path in its EROFS stderr; failures that omit the path retain the generic nonzero-exit diagnostic.