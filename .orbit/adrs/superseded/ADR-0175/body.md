**Context.** Phase 1 used the semantic name for the hybrid BM25 plus cosine mode toggle and a separate related-task flag for cosine-neighbor lookup. That inverted the intuitive reading of semantic search: users expect semantic plus an ID to mean nearest neighbors, while hybrid is the honest name for the ranking algorithm.

**Decision.** Rename the free-text ranking toggle to `--hybrid` / `hybrid: true` and rename task-neighbor lookup to `--semantic <id>` / `semantic: "<id>"`. Keep lexical search as the default and report JSON mode `hybrid` for hybrid free-text search and `neighbor` for cosine-only task-neighbor lookup.

**Consequences.**
- The CLI and MCP surfaces match user vocabulary before external consumers depend on the phase-1 names.
- Historical phase-1 audit payloads that carried `semantic: true` are orphaned by the hard break, matching the no-shim policy for this young surface.
- Documentation and packaged skills must distinguish the `orbit semantic` lifecycle command from the MCP `semantic: "<id>"` search parameter. ADR-0179 replaces the CLI flag form with `orbit search similar <id>`.
- Cost: Agents and docs written against phase 1 need a one-time rename sweep, and ORB-00202 may need a rebase because it edits adjacent search surfaces.
- Cost: historical audit event names `semantic.search` and `semantic.related` become orphaned event types, accepted because no external audit-history consumers exist yet.