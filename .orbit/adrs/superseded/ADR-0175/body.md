## Context
Phase 1 shipped `orbit search --semantic` as the hybrid BM25 plus cosine mode toggle and `orbit search --related <id>` as the cosine-neighbor task lookup. That inverted the intuitive reading of semantic search: users expect semantic plus an ID to mean nearest neighbors, while hybrid is the honest name for the ranking algorithm.

## Decision
Rename the free-text ranking toggle to `--hybrid` / `hybrid: true` and rename task-neighbor lookup to `--semantic <id>` / `semantic: "<id>"`. Keep lexical search as the default and report JSON mode `hybrid` for hybrid free-text search and `neighbor` for cosine-only task-neighbor lookup.

## Consequences
- The CLI and MCP surfaces match user vocabulary before external consumers depend on the phase-1 names.
- Historical phase-1 audit payloads that carried `semantic: true` are orphaned by the hard break, matching the no-shim policy for this young surface.
- Documentation and packaged skills must distinguish the `orbit semantic` lifecycle command from the `--semantic <id>` search flag.
- Cost: Agents and docs written against phase 1 need a one-time rename sweep, and ORB-00202 may need a rebase because it edits adjacent search surfaces.