## Context

Planning-duel artifacts and scoreboards compared model strings even though model names drift across aliases, CLI shorthand, and self-reported tool payloads. A Gemini planner configured as `pro` could produce an artifact stamped `gemini-3.1-pro`; both values describe the same family but failed equality checks. Alias tables (`resolve_agent_model_pair*`, `matches_model_alias`, `canonical_model_for_agent`) treated the symptom and grew with every provider change.

## Decision

Family is identity, model is configuration, and slot is role. Orbit identity surfaces use exactly `codex`, `claude`, `gemini`, or `grok`. Planning-duel assignments persist `family`; `planner_a`, `planner_b`, and `arbiter` are explicit slots used in artifact paths and signatures. Exact model strings stay in crew config, `[duel.models]`, CLI invocation translation, and resolved-crew run records.

## Consequences

- New planning-duel artifacts are written as `planning-duel/{slot}.md` and signed `*authored by: {family} / {slot}*`; historical model-path artifacts remain a legacy read concern.
- Runtime tool boundaries treat envelope identity as authoritative. Agent-supplied `model` fields are overwritten with the canonical family before persistence/comparison so self-report drift cannot affect validation.
- Scoreboard and friction projections are family-keyed (`by_family`) when they answer "who actually ran?". Resolved-crew projections remain the source for "who was selected?" because they describe configured routing.
- The legacy resolver and alias-canonicalization surfaces are deleted from production code. `infer_agent_family_from_model` remains for legacy artifact recovery and CLI invocation translation.
- ORB-00079 and ORB-00071 are superseded by this structural identity change.
- Cost: model granularity is lost from identity comparisons. Two different Gemini model versions (e.g. `pro` vs `flash`) collapse to the same `gemini` identity in scoreboards; distinguishing them requires drilling into resolved-crew run records or `[duel.models]` configuration.