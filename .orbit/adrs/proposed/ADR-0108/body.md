## Context
Three classes of discovery were on the table:

| Approach | Profile |
|----------|---------|
| **Pull-only via search tool** | An `orbit.learning.search` MCP tool. Agents query when they think to. Lowest implementation cost; depends entirely on agent discipline. |
| **Push at session start** | All learnings (or an agent-curated subset) load into agent context at session start, like `CLAUDE.md` does. No discipline required, but unscoped and noisy at scale. |
| **Push at the moment of action** | Scoped injection triggered by the file path or task an agent is about to touch. Higher implementation cost; matches discoverability cost to relevance value. |

The repeated failure mode the system exists to prevent is *agents not knowing they should look*. Pull-only inherits that failure mode wholesale: the agent that needed the learning most — the one that forgot the rule — is the one who won't think to query. Session-start push avoids the discipline problem but punishes every session with content that may not apply.

## Decision
Phase 1 ships push-at-the-moment-of-action across three layers: engine pre-prompt injection (universal, task-scoped), MCP tool-response sidecar (cross-agent, file-path-scoped), and Claude Code `PreToolUse` hook (Claude Code only, edit-scoped). A pull surface (`orbit.learning.search`, `orbit-learnings` skill) ships alongside as a complement, not a substitute.

## Consequences
- Agents get relevant learnings without having to query — the discoverability failure mode is closed.
- Authoring effort produces compounding value: every learning is delivered the next time anyone touches the relevant area, automatically.
- The three-layer architecture means coverage degrades gracefully: agents without hook support still get layers 1 and 2.
- Cost: every Orbit-spawned task and every relevant MCP tool call pays a small latency hit for the scope-match query, plus a few dozen tokens of context per injected learning. At expected scale (low hundreds of learnings, sub-millisecond match) the latency is negligible; the context cost is bounded by the per-call cap of 5 and the per-session cap of 20. The cost is real and paid uniformly — even on tasks where no learning applies, the engine still queries to find that out.

---
