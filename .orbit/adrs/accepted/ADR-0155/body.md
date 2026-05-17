## Context

Duel-plan previously walked the full `all_agent_families()` registry and used the same model-pair resolution chain as non-duel callers. That made local CLI availability load-bearing for every supported family and made reproducible planning-duel scoreboards depend on executor YAML state.

## Decision

Add a workspace `[duel]` section with `candidates` as a normalized subset of `all_agent_families()` and `[duel.models]` as flat orchestrator-only per-family overrides. Duel role selection reads those values through `RuntimeHost`; non-duel callers continue to use executor overrides and builtin model pairs.

## Consequences

- Duel permutations remain dynamic but require at least three distinct configured families.
- `[duel.models]` wins only for duel role-model lookup; helper models and non-duel model identity are unchanged.
- The crew registry remains separate from duel participant selection. Reusing `[crews.*]` for duels was rejected because duels need a family pool, not a fixed planner/implementer/reviewer lineup.
- Cost: duel-plan reproducibility now depends on a third configuration surface (`[duel]`) in addition to crew registry and executor overrides. Operators triaging a duel run must consult all three to explain a given family/model selection.