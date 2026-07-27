## Context
Orbit shares one workspace store and documentation index across linked Git worktrees. Letting `orbit docs list`, `show`, or `index` overlay the caller's worktree would make the same shared index return different content based on invocation directory and would introduce last-writer-wins races between concurrent task worktrees. The alternative was a per-worktree index or a worktree-first fallback such as ORB-10504.

## Decision
The primary checkout's resolved shared workspace root is the only authoritative source for the Orbit documentation corpus and index. `orbit docs index`, `list`, and `show` must not overlay or fall back to caller-worktree document content, and Orbit must not create a second per-worktree documentation index. The generated human-facing `docs/INDEX.md` likewise remains a single canonical artifact at the primary documentation root. Agents validate unmerged worktree documentation by reading the files directly and running the repository's frontmatter, generator, and freshness checks; those edits become visible through the documentation index only after they land in the primary checkout and the index is refreshed.

## Consequences
- Every caller observes one reproducible documentation corpus, independent of its current linked worktree.
- Concurrent task worktrees cannot overwrite or shadow one another in the shared docs index.
- Worktree validation uses source files and deterministic checks rather than treating a successful shared index refresh as proof of unmerged content.
- Cost: `orbit docs show` and search cannot preview unmerged worktree-only document edits; those edits must be inspected directly until they land in the primary checkout.