## Context

Workspace config previously selected planner, implementer, and reviewer models with three top-level `[agent.<role>]` tables, while task execution had no durable way to request a different lineup. Layering a new registry beside the old role tables would have forced Orbit to validate and explain two schemas for the same decision.

## Decision

Replace the role-keyed config shape wholesale with named `[crews.<name>]` entries and `[workflow].default_crew`. A task may store `crew`, and a run may override it with CLI/tool input; precedence is CLI override, then task field, then workspace default.

## Consequences

- "Crew" was chosen over "profile" because profiles sound user-scoped, and over "pair" because the lineup contains planner, implementer, and reviewer.
- Run records persist the resolved crew plus the three role model strings so audit trails survive later config edits.
- The v2 `agent_loop` dispatch path reads role models from the crew registry (`crates/orbit-core/src/runtime/engine/environment_host.rs`). Scoreboard and friction projections use family identity; exact model strings remain visible through resolved crew/run configuration.
- Deferred: duel-plan participant configuration, per-role task overrides, and planner-vs-executor workflow split.
- Cost: old workspaces with only `[agent.planner]`, `[agent.implementer]`, and `[agent.reviewer]` must migrate before config load succeeds.