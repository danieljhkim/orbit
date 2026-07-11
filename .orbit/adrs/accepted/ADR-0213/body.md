## Context
Named crews currently carry separate planner, implementer, and reviewer assignments, but production crews are homogeneous and only the implementer is on the primary ship path. Role labels still matter for prompts and telemetry, while model selection through three independent slots adds configuration and persistence complexity without selecting distinct behavior.

## Decision
A crew is one provider-model-backend assignment. Every activity role resolves to that assignment; role labels remain descriptive, duel participant selection stays independent, and legacy three-role config is accepted by choosing implementer while warning when the discarded roles diverge.

## Consequences
- Per-task and default crew selection now directly choose one provider-model binding.
- Run records and projections expose one crew model; legacy SQLite role columns remain nullable for compatibility.
- Existing homogeneous legacy crew configuration continues to load without behavior changes.
- Cost: Deliberately heterogeneous legacy crews collapse to their implementer assignment and require a warning-guided config rewrite; cross-provider review must use duel machinery or a future explicit mechanism.