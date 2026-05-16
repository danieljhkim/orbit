# Agent Families — Decisions

**Status:** Draft
**Owner:** grok
**Last updated:** 2026-05-16

ADR entries are append-only and ordered ascending. New entries should follow the template in [../CONVENTIONS.md](../CONVENTIONS.md).

## ADR-0151 (2026-05-16)

**Title:** Add Grok (xAI) as a fourth peer agent family

**Decision:** Treat "grok" as a full peer alongside claude, codex, and gemini.

**Key Changes:**
- Extended `agent_from_model()`, `infer_agent_family_from_model()`, `all_agent_families()`, `resolve_agent_model_pair()`, and `provider_from_model()`
- Added `grok.yaml` executor skeleton and sandbox support (tasks ORB-00044, ORB-00045)
- Added Grok provider to `orbit mcp init` (ORB-00046)
- Created this design doc folder (ORB-00052)

See full ADR-0151 for context, alternatives considered, and cost analysis.

## ADR-0152 — Replace `[agent.<role>]` tables with named `[crews.*]` registry

**Status:** Accepted · 2026-05 · [ORB-00058]

**Context.** Workspace config previously selected planner, implementer, and reviewer models with three top-level `[agent.<role>]` tables, while task execution had no durable way to request a different lineup. Layering a new registry beside the old role tables would have forced Orbit to validate and explain two schemas for the same decision.

**Decision.** Replace the role-keyed config shape wholesale with named `[crews.<name>]` entries and `[workflow].default_crew`. A task may store `crew`, and a run may override it with CLI/tool input; precedence is CLI override, then task field, then workspace default.

**Consequences.**
- "Crew" was chosen over "profile" because profiles sound user-scoped, and over "pair" because the lineup contains planner, implementer, and reviewer.
- Run records persist the resolved crew plus the three role model strings so audit trails survive later config edits.
- The v2 `agent_loop` dispatch path reads role models from the crew registry (`crates/orbit-core/src/runtime/engine/environment_host.rs`). The v1 envelope/identity path and the orbit-store scoreboard/friction projections still resolve through `resolve_agent_model_pair*` in `crates/orbit-common/src/types/agent_pair.rs`; those callers do not yet have access to a crew-aware context, so this PR keeps the legacy resolver as a scoreboard-rendering shim. Migrating them is tracked as a follow-up.
- Deferred: duel-plan participant configuration, per-role task overrides, planner-vs-executor workflow split, and the legacy-resolver migration noted above.
- Cost: old workspaces with only `[agent.planner]`, `[agent.implementer]`, and `[agent.reviewer]` must migrate before config load succeeds.

## Task References

- ORB-00042: Onboard Grok (xAI) as a first-class supported agent family.
- ORB-00058: Introduce per-task crew override for agent model selection.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
