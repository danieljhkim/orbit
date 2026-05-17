## Context
The push-injection layer ([2_design.md §4](./2_design.md)) has multiple natural placements, each with different coverage:

- **Engine pre-prompt only.** Inject when `orbit-engine` spawns an agent for a task. Universal across agents. Coarse: fires once at task start, before the agent has read its way to the relevant code, so narrow learnings (file-path-scoped) may not surface for the file the agent edits ten tool calls in.
- **MCP-sidecar only.** Attach `learnings` to MCP tool responses that reference paths. Cross-agent. Misses Claude Code's built-in `Edit | Write | Read`, which agents use far more than they call MCP file tools.
- **Claude Code `PreToolUse` only.** Per-edit precision. Vendor-locked: doesn't apply to Codex, Gemini, Anthropic-API, Ollama, or any other agent runtime.
- **All three layered.** Each layer adds precision on top of the layers below. Coverage degrades gracefully: agents without hook support still get layers 1 and 2; tools without path arguments still get layer 1.

The vendor-locked single-layer options are non-starters because the project supports multiple agent providers (see `crates/orbit-agent/providers/`). Engine-pre-prompt-only misses the long-task case where an agent works for an hour through a wide context. MCP-sidecar-only misses the most-frequent agent action (built-in editor tools).

## Decision
Phase 1 ships all three layers active simultaneously. Each layer consults a per-session deduplication set so the same learning doesn't inject multiple times across layers. Per-call cap of 5 learnings; per-session cap of 20.

## Consequences
- Coverage is robust: even if one layer misfires or a vendor lacks hook support, the others provide a baseline.
- Agents see relevant learnings at multiple natural moments — task start, MCP tool call, individual edit — without being drowned in repeats (dedup set).
- The architecture admits a future "layer 4" (Orbit-side proxy for agents without hooks) without restructuring, but doesn't require it ([3_vision.md §1.5](./3_vision.md)).
- Cost: three injection sites means three places to maintain. A schema change to learning records (new field surfaced at injection time) requires touching `orbit-engine`, `orbit-mcp`, and the Claude Code hook script. The dedup set is agent-local; if context is compressed mid-session, the set may reset and the same learning may inject twice. Both costs are accepted as the price of robust coverage; collapsing to a single layer would mean choosing one failure mode (vendor lock-in, coarse scope, or missing built-in tools) and living with it.

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
