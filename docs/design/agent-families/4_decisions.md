# Agent Families — Decisions

**Status:** Draft
**Owner:** grok
**Last updated:** 2026-05-17

ADR entries are append-only and ordered ascending by global ID. New entries are allocated via `orbit.adr.add` *before* the local heading is written — see [../CONVENTIONS.md §4](../CONVENTIONS.md) and the `orbit-adr` skill. The local heading uses the allocated global ID verbatim.

Historical note: prior to 2026-05-17, ADR-0154 / ADR-0155 / ADR-0156 in this file were authored with locally-invented IDs (`ADR-0152` / `ADR-0153` / `ADR-0154`) that did not match the global store. They were re-allocated through `orbit.adr.add` per [ORB-00098]; the original local IDs survive as `legacy_ids` so prior citations still resolve.

## ADR-0151 — Add Grok (xAI) as a fourth peer agent family

**Status:** Accepted · 2026-05-16 · [ORB-00042] · [ORB-00043] · [ORB-00044] · [ORB-00045] · [ORB-00046] · [ORB-00052] · legacy_id: `agent-families/ADR-0151`

**Decision:** Treat "grok" as a full peer alongside claude, codex, and gemini.

**Key Changes:**
- Extended `agent_from_model()`, `infer_agent_family_from_model()`, `all_agent_families()`, `resolve_agent_model_pair()`, and `provider_from_model()`
- Added `grok.yaml` executor skeleton and sandbox support (tasks ORB-00044, ORB-00045)
- Added Grok provider to `orbit mcp init` (ORB-00046)
- Created this design doc folder (ORB-00052)

See full ADR-0151 for context, alternatives considered, and cost analysis.

## ADR-0154 — Replace `[agent.<role>]` tables with named `[crews.*]` registry

**Status:** Accepted · 2026-05 · [ORB-00058] · legacy_id: `agent-families/ADR-0152`

**Context.** Workspace config previously selected planner, implementer, and reviewer models with three top-level `[agent.<role>]` tables, while task execution had no durable way to request a different lineup. Layering a new registry beside the old role tables would have forced Orbit to validate and explain two schemas for the same decision.

**Decision.** Replace the role-keyed config shape wholesale with named `[crews.<name>]` entries and `[workflow].default_crew`. A task may store `crew`, and a run may override it with CLI/tool input; precedence is CLI override, then task field, then workspace default.

**Consequences.**
- "Crew" was chosen over "profile" because profiles sound user-scoped, and over "pair" because the lineup contains planner, implementer, and reviewer.
- Run records persist the resolved crew plus the three role model strings so audit trails survive later config edits.
- The v2 `agent_loop` dispatch path reads role models from the crew registry (`crates/orbit-core/src/runtime/engine/environment_host.rs`). Scoreboard and friction projections use family identity after ADR-0156; exact model strings remain visible through resolved crew/run configuration.
- Deferred: duel-plan participant configuration, per-role task overrides, and planner-vs-executor workflow split.
- Cost: old workspaces with only `[agent.planner]`, `[agent.implementer]`, and `[agent.reviewer]` must migrate before config load succeeds.

## ADR-0155 — Scope duel-plan candidate and model overrides to `[duel]`

**Status:** Accepted · 2026-05 · [ORB-00072] · legacy_id: `agent-families/ADR-0153`

**Context.** Duel-plan previously walked the full `all_agent_families()` registry and used the same model-pair resolution chain as non-duel callers. That made local CLI availability load-bearing for every supported family and made reproducible planning-duel scoreboards depend on executor YAML state.

**Decision.** Add a workspace `[duel]` section with `candidates` as a normalized subset of `all_agent_families()` and `[duel.models]` as flat orchestrator-only per-family overrides. Duel role selection reads those values through `RuntimeHost`; non-duel callers continue to use executor overrides and builtin model pairs.

**Consequences.**
- Duel permutations remain dynamic but require at least three distinct configured families.
- `[duel.models]` wins only for duel role-model lookup; helper models and non-duel model identity are unchanged.
- The crew registry remains separate from duel participant selection. Reusing `[crews.*]` for duels was rejected because duels need a family pool, not a fixed planner/implementer/reviewer lineup.

## ADR-0156 — Collapse agent identity to family and move model strings to configuration

**Status:** Accepted · 2026-05 · [ORB-00080] · legacy_id: `agent-families/ADR-0154`

**Context.** Planning-duel artifacts and scoreboards compared model strings even though model names drift across aliases, CLI shorthand, and self-reported tool payloads. A Gemini planner configured as `pro` could produce an artifact stamped `gemini-3.1-pro`; both values describe the same family but failed equality checks. Alias tables (`resolve_agent_model_pair*`, `matches_model_alias`, `canonical_model_for_agent`) treated the symptom and grew with every provider change.

**Decision.** Family is identity, model is configuration, and slot is role. Orbit identity surfaces use exactly `codex`, `claude`, `gemini`, or `grok`. Planning-duel assignments persist `family`; `planner_a`, `planner_b`, and `arbiter` are explicit slots used in artifact paths and signatures. Exact model strings stay in crew config, `[duel.models]`, CLI invocation translation, and resolved-crew run records.

**Consequences.**
- New planning-duel artifacts are written as `planning-duel/{slot}.md` and signed `*authored by: {family} / {slot}*`; historical model-path artifacts remain a legacy read concern.
- Runtime tool boundaries treat envelope identity as authoritative. Agent-supplied `model` fields are overwritten with the canonical family before persistence/comparison so self-report drift cannot affect validation.
- Scoreboard and friction projections are family-keyed (`by_family`) when they answer "who actually ran?". Resolved-crew projections remain the source for "who was selected?" because they describe configured routing.
- The legacy resolver and alias-canonicalization surfaces are deleted from production code. `infer_agent_family_from_model` remains for legacy artifact recovery and CLI invocation translation.
- ORB-00079 and ORB-00071 are superseded by this structural identity change.

## Task References

- ORB-00042: Onboard Grok (xAI) as a first-class supported agent family.
- ORB-00058: Introduce per-task crew override for agent model selection.
- ORB-00072: Make duel-plan agent pool and per-family model configurable via `[duel]`.
- ORB-00080: Collapse agent identity to family; isolate model strings to invocation surface.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
