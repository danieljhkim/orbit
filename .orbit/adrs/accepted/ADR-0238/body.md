## Context

The 2026-07-18 relevancy audit (F2026-07-092) showed the learning PreToolUse hook fired 2,374 times over two weeks with 13 injections and zero usage signal: nothing recorded whether an injected learning shaped the receiving agent's work, so nothing could drive deprecation of stale learnings. ADR-0210 had removed the earlier vote/comment feedback surfaces for lack of real usage, explicitly leaving the door open for 'a scoped feedback primitive ... reintroduced with real usage data behind it.' Separately, per-session injection dedup was dead in interactive sessions: ORBIT_SESSION_ID was exported on 0/2,374 fires and the ppid-tmpfile fallback re-keys per invocation.

## Decision

1. Record the feedback signal as `learning_ack` audit events in the existing host-global audit store (`~/.orbit/orbit.db`), not as a new sidecar or store: `target_id` = learning ID, `arguments_json.outcome` = `used` | `ignored`. Recorded via `orbit learning ack <id>... [--ignored]` or the `orbit.learning.ack` MCP tool (an additive entry in the frozen mcp-bridge conformance-v1 fixture). The injected reminder block documents the ack call inline.
2. Absent ack counts as **ignored**. The rollup (`orbit learning stats`, folding `learning_injected` + `learning_ack` events per learning) derives ignored = injected − used, so a silent agent population biases learnings toward deprecation, never away from it.
3. Instrumentation fails open. An unavailable audit backend logs a warning and injection still renders; `orbit learning ack` warns and exits 0 on backend failure (caller mistakes such as unknown IDs still fail closed).
4. Session dedup keys on the first resolvable anchor: ORBIT_SESSION_ID env (engine-managed runs, pre-seeded with layer-1 injections) → the `session_id` field carried by the hook payload itself (Claude Code sends it on every hook event) → ppid-tmpfile last resort.

Rejected alternatives: a dedicated ack table or per-learning sidecar (disproportionate infrastructure for an observability signal — the exact failure mode ADR-0210 removed); treating absent ack as used (optimistic default would make the rollup useless for deprecation); a session-end Stop-hook auto-ack (cannot know used-ness automatically; a bulk auto-ack would fabricate the signal).

## Consequences

- The rollup is the designed input for downstream deprecation policy; decay/TTL is deliberately follow-up work, not implemented at this layer.
- The `learning_ack` contract lives in audit-event conventions (target_type + arguments_json), enforced by store-level fold tests rather than a schema migration.
- Used-ratio data is only as good as agent ack discipline; the ignored-by-default rule makes the failure mode conservative (over-deprecation pressure, surfaced before any automated deprecation exists).
- Cost: one extra reminder-block line per injection and one audit row per ack; the stats fold scans only learning-targeted audit rows.