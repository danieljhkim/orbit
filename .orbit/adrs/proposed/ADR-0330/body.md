## Context

ADR-0213 flattened a crew to one provider-model-backend assignment but kept two compatibility surfaces: legacy `planner`/`implementer`/`reviewer` sub-tables accepted at config load, and an `AgentRole` label carried on activities and job steps. Both are now inert. A legacy crew must supply all three sub-tables, and load keeps the implementer assignment while discarding the other two behind a warn-level log when they diverge; role-based resolution returns the run's single crew assignment for every role, so an activity declaring `role: reviewer` runs on the run's resolved crew. Three routing mechanisms are documented; only an explicit `crew` input actually selects a different model, and a declared role currently pre-empts it. The alternative was to keep the shims indefinitely, at the cost of a schema that advertises selection it does not perform.

This amends two clauses of ADR-0213: that legacy three-role config is accepted by choosing implementer with a warning, and that role labels remain as descriptive resolution inputs. ADR-0213's core decision — one assignment per crew — stands and is completed here.

## Decision

Crew configuration accepts flat `provider`/`model`/`backend` only; the legacy role sub-tables are rejected at load with rewrite guidance rather than collapsed. `AgentRole` and the role-to-assignment resolution path are removed from config, runtime, and asset schemas. Routing becomes: an activity or step with an explicit `crew` input dispatches on that crew, and one without dispatches on the run's resolved crew — replacing today's inline-baseline fallback, so no activity is left without a crew when its role is removed.

## Consequences

- Two routing mechanisms collapse to one, so an activity's crew is either named in its input or is the run's.
- A heterogeneous legacy crew now fails loudly at load instead of losing its planner and reviewer assignments behind a log line.
- System activities that need a model distinct from the run's crew must name a configured crew through an input, making that choice visible in config rather than in engine code.
- Cost: breaking config change. Any workspace carrying the three-role shape fails to load until rewritten, and shipped activity assets carrying `role:` must be reseeded in the same release.
- Cost: the planning duel still overrides provider and model at dispatch time through its own override path, so a duel activity's asset alone does not tell you which model ran it. That carve-out, inherited from ADR-0213, is unchanged here.