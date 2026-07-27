## Context

The 2026-07-18 relevancy audit (friction F2026-07-092) found the learning PreToolUse hook fired 2,374 times over two weeks with 13 injections (0.55%) and **zero usage signal**: nothing recorded whether an injected learning shaped the receiving agent's work, so nothing could drive deprecation of stale learnings. ADR-0210 removed the vote/comment feedback surfaces for lack of real usage, ending with an explicit reopening clause: a scoped feedback primitive can return "with real usage data behind it." The audit is that data.

A first attempt (PR #657, closed unmerged) added an explicit `orbit learning ack` CLI/MCP surface with ignored-by-default semantics. Daniel rejected it: an ack is an active, gameable step the agent must remember to take, it costs a reminder-block footer line and a new MCP tool in the frozen conformance surface, and "unacked = ignored" biases every silent session toward deprecation regardless of whether the learning was actually useless.

Separately, per-session injection dedup was dead in interactive sessions — `ORBIT_SESSION_ID` was exported on 0/2,374 observed fires, and the ppid-tmpfile fallback re-keys per invocation because every hook fire runs under a fresh parent shell (L-0077 injected 10× in one session).

Alternatives considered:

| Approach | Profile |
|----------|---------|
| **Explicit `learning ack` surface (PR #657)** | Active, gameable, adds an MCP tool to the frozen conformance fixture and a reminder footer line; silence forced to mean "ignored." Rejected by Daniel. |
| **Full-content injection + no signal (status quo)** | High token cost per fire, no usage data, no deprecation input. |
| **Teaser injection + show-as-signal (this ADR)** | Injection carries only id + summary + tags; opening the full body via `orbit learning show` is the passive, ungameable usage signal. Lower token cost, no new agent action, no new MCP tool. |

## Decision

1. **Teaser injection.** The injection layers project only the learning id, one-line summary, and scope tags into agent context (`render_reminder_block`). The full body is retrieved on demand via `orbit learning show <id>` — the reminder block already tells the agent how. This drops per-fire injection token cost and makes "read the full learning" an explicit, observable act.

2. **Show-as-usage-signal.** `orbit learning show` (CLI and `orbit.learning.show` MCP tool) records a `learning_shown` audit event in the host-global `~/.orbit/orbit.db`, keyed by learning id + session, alongside the existing `learning_injected` events. It is the passive signal: an agent that opens a learning found the teaser worth expanding. No ack, no new tool, no schema change — `orbit.learning.show` already exists.

3. **Aggregation.** `orbit learning stats` folds `learning_injected` + `learning_shown` per learning into injected count, shown count, shown ratio, and last-injected/last-shown timestamps (CLI + `learning_usage_stats` runtime API). This rollup is the designed input for the downstream deprecation sweep (ORB-10318); a low shown ratio (injected often, never read) is the deprecation-candidate signal. No deprecation logic lives here.

4. **Fail-open instrumentation.** An unavailable audit backend logs a warning and injection still renders; `learning show` logs a warning and still returns the learning when the `learning_shown` emit fails. The signal is best-effort observability and must never break the read or injection path.

5. **Session dedup** keys on the first resolvable anchor: `ORBIT_SESSION_ID` env (engine-managed runs export it, pre-seeded with layer-1 injections) → the `session_id` field the hook payload itself carries (Claude Code sends it on every hook event) → ppid-tmpfile last resort.

**No ack surface.** There is deliberately no `orbit learning ack` CLI, no `orbit.learning.ack` MCP tool, and no ack instruction in the injected block.

## Consequences

- The rollup is the designed input for downstream deprecation policy (ORB-10318); decay/TTL is deliberately follow-up work, not implemented here.
- The `learning_shown` / `learning_injected` contract lives in audit-event conventions (`target_type` + `arguments_json`) enforced by store-level fold tests, not a schema migration — consistent with the injection events it joins against.
- Signal quality depends on agents actually opening learnings they use, but `show` is far harder to game than an ack and costs the agent nothing extra to emit; the ratio is directional input for a human/automated sweep, not an automated gate.
- Cost: one audit row per `show`, plus scope tags added to each teaser line. No change to the MCP conformance surface (no new tool), unlike the rejected ack design.