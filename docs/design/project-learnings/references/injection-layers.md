# Learning Delivery and Discovery

**Last updated:** 2026-07-25

This legacy-named reference records the current delivery model after automatic learning injection was retired. It is a quick companion to [2_design.md §4](../2_design.md): look here to choose a retrieval surface; look there for the full design and the reference-comment convention.

## Surfaces

Orbit exposes the same logical tool registry (`orbit.<group>.<action>`) through two transports:

- **MCP** — `orbit mcp serve`, surfaced as `mcp__orbit__*` tools to agents speaking MCP (Claude Code, Codex, Gemini CLI).
- **CLI** — `orbit tool run orbit.<group>.<action> --input '<json>'`, used inside engine-spawned activity envelopes and from human shells.

Same tools, same JSON I/O. Neither transport injects a learning reminder into an unrelated tool call.

## Coverage matrix

| Agent context | Discovery | Retrieval | Point-of-use locator |
|---|---|---|---|
| Engine-spawned agent in worktree | `orbit search --kind learning` | `orbit learning show <id>` | nearby reference comment when one exists |
| Interactive MCP client | `orbit.search` with `kind: "learning"` | `orbit.learning.show` | nearby reference comment when one exists |
| Human or programmatic CLI caller | `orbit search --kind learning` | `orbit learning show <id>` | documentation or a nearby reference comment |

The model is intentionally the same for every agent vendor and transport. Search returns candidates; show reads the authoritative body and records `learning_shown`.

## Rules for discovery

1. Use `orbit.search` / `orbit learning list` to find candidate records, then `show` the full record only when it is relevant.
2. Put a short `L-NNNN` or `ADR-NNNN` reference comment at a durable code or workflow boundary when it would help the next reader find the rationale. Do not copy the body there.
3. Do not put workspace-local artifact IDs in shipped prompts, skill text, or other consumer-facing instruction surfaces: IDs are local to the artifact registry and can dangle in another workspace.
4. Do not restore a PreToolUse, pre-prompt, or MCP-sidecar reminder without new evidence and a dedicated decision. The prior injection data is frozen as of 2026-07-20.

## See also

- [2_design.md §4](../2_design.md) — pull delivery and reference-comment convention.
- [2_design.md §8.4](../2_design.md) — tradeoff: pull delivery needs a useful locator.
- [4_decisions.md](../4_decisions.md) — historical and current delivery decisions.
- [glossary.md](./glossary.md) — terminology used here and in §4.

## Code anchors

Current delivery is exposed through `orbit search`, `orbit learning list`, and `orbit learning show` (or their registered tool equivalents). There is no repository learning-reminder hook registration.
