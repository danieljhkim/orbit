## Context

Orbit dispatches agent runs across provider families through named crews, and any crew can be assigned to any lane. Turn caps are not portable: provider CLIs expose different controls and semantics. Real alternatives were to retain optional provider-specific turn budgets, or forbid them as cross-provider policy and use neutral limits.

## Decision

All run budgets in Orbit config, auto-task definitions, job/activity assets, workflow invoke payloads, and task specs are provider-neutral: wall-clock timeout is the primary bound, with neutral resource caps where needed. Turn caps (`max_turns` or equivalents) do not appear on these surfaces. Provider-specific throttles may exist only inside one provider adapter as implementation detail.

## Consequences

- Swapping a crew never changes configured budget semantics.
- Config and job assets stay portable across crews without provider conditionals in dispatch paths.
- Existing turn-based policy knobs must be retired or demoted to adapter-internal defaults.
- Cost: Orbit gives up fine-grained turn limits exposed by individual providers; a looping run is bounded by neutral time/resource limits instead.