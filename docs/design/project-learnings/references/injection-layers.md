# Injection-Layer Coverage Matrix

**Last updated:** 2026-05-17

A coverage map of the project-learnings push-injection pipeline against Orbit's two agent tool surfaces. Quick-reference companion to [2_design.md §4](../2_design.md), which is the design itself. Look here to answer "does *my* surface get learning X?"; look there to understand how each layer is built.

## Surfaces

Orbit exposes the same logical tool registry (`orbit.<group>.<action>`) through two transports:

- **MCP** — `orbit mcp serve`, surfaced as `mcp__orbit__*` tools to agents speaking MCP (Claude Code, Codex, Gemini CLI).
- **CLI** — `orbit tool run orbit.<group>.<action> --input '<json>'`, used inside engine-spawned activity envelopes and from human shells.

Same tools, same JSON I/O. The transports differ only in what wraps the response.

## Coverage matrix

| Agent context | Transport | L1 pre-prompt (universal) | L2 MCP sidecar (cross-agent) | L3 PreToolUse hook (Claude-only) |
|---|---|---|---|---|
| Engine-spawned agent in worktree | CLI | **✓** | ✗ (not MCP) | rare in-envelope |
| Interactive Claude Code with Orbit plugin | MCP | ✗ (no engine envelope) | **✓** | **✓** (on `Edit`/`Write`/`Read`) |
| Interactive Codex / Gemini with Orbit MCP | MCP | ✗ | **✓** | ✗ (no equivalent hook surface) |
| Human at shell | CLI | ✗ | ✗ | ✗ |
| Other programmatic CLI caller | CLI | ✗ | ✗ | ✗ |

Three observations:

1. **Every cell that should be covered, is.** L1 covers the engine-driven path. L2 covers MCP. L3 covers Claude Code's file-touch surface. Humans and scripts intentionally get nothing — auto-injection in a shell is noise, not signal.
2. **L2 lives in `orbit-mcp`, not in the tool layer.** That is correct; see Rule 2 below.
3. **Uneven coverage by agent vendor is by design.** [§8.4](../2_design.md) accepts the unevenness because L1 is universal and forms a baseline that is strictly better than today.

## Rules for new enrichments

A "new enrichment" is anything that decorates a tool response, adds a sidecar field, or extends pre-prompt context. The decision is **which layer**, not **which surface**.

### Rule 1 — Place by consumption mode, not by data source

- Enrichment about *prompt context for an agent* → push layer (L1 / L2 / L3 or a new one).
- Enrichment about *what the tool returns to any caller* → tool layer (`orbit-tools` / `orbit-core` host actions).

Useful test: would a human running `orbit tool run ...` in a shell want to see this field? If yes → tool layer. If no → push layer.

### Rule 2 — Sidecar enrichments live in the adapter that owns the session

L2 lives in `orbit-mcp` because:

- Session dedup needs session state. MCP has sessions; the tool layer does not. Pushing dedup down requires threading a session ID through every tool call.
- Caps and admission policy are consumer-shape concerns. Different consumers (MCP, CLI, future REST adapter) tolerate different volumes of injected context.
- ARCHITECTURE.md forbids `orbit-mcp → orbit-store`, so L2 re-enters the tool surface via `McpHost::call_tool("orbit.search", …)` with a `{"kind": "learning"}` body. The tool layer is the *callee*, not the *home*, of the sidecar.

If a future adapter (REST, gRPC, …) needs its own sidecar, the right move is **another L2-shaped layer in that adapter**, not pulling the existing one down into `orbit-tools`.

### Rule 3 — Maintain the canonical-data invariant

Both transports MUST return the same canonical data for the same tool. Enrichment is allowed (adapter-side `learnings: [...]` field on the response); divergent core data is not.

- ✓ MCP response adds a `learnings` sidecar that CLI lacks.
- ✗ MCP response omits a field that CLI returns, or returns it under a different name.

This invariant is what lets agents and humans use either surface interchangeably for the underlying capability. If you find yourself wanting to violate it, you have probably misclassified the change under Rule 1.

## Known gap: out-of-envelope CLI

The bottom rows of the matrix (human at shell, programmatic caller) have no injection layer. Today this is intentional — neither audience benefits from auto-injection. If a future use case appears (e.g. a CI runner wanting learning context in logs), the right mechanism is an opt-in flag on `orbit tool run` such as `--with-learnings`, **not** unconditional wrapping in the tool layer. Opt-in keeps Rule 3 intact and forces the caller to declare consent.

## See also

- [2_design.md §4](../2_design.md) — push-injection pipeline design, layer-by-layer.
- [2_design.md §8.4](../2_design.md) — design rationale for uneven coverage by agent vendor.
- [4_decisions.md](../4_decisions.md) — accepted ADRs for the injection layers.
- [glossary.md](./glossary.md) — terminology used here and in §4.

## Code anchors

Current as of the `Last updated` date above. Re-grep before relying on these if the date is older than a release.

- [`crates/orbit-mcp/src/adapter/learning_sidecar.rs`](../../../../crates/orbit-mcp/src/adapter/learning_sidecar.rs) — L2 implementation (allowlist, path collection, session admission, response attachment).
- [`crates/orbit-mcp/src/adapter/dispatch.rs`](../../../../crates/orbit-mcp/src/adapter/dispatch.rs) — call site that wraps every MCP tool response.
- `crates/orbit-engine/...` — L1 implementation (engine pre-prompt injection at agent runtime spawn).
- `.claude/settings.json` (per-user) — L3 PreToolUse hook configuration.
