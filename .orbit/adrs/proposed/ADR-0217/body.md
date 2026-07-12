## Context

Orbit dispatches agent runs across provider families (claude, codex, gemini, grok) via named crews [ORB-10130], and any crew can be assigned to any lane. Turn caps are not a portable control: `--max-turns` is a claude-only CLI flag (codex rejects the control; the worker daemon's `SUPERVISOR_MAX_TURNS` is documented claude-only). Budgets expressed in turns therefore silently change meaning — or break — when a task's crew changes. This surfaced in ORB-10146 (rejected), whose spec embedded a "~150 turns" budget and whose implementation had to special-case "send max_turns only for claude", i.e. a cross-provider policy knob only one provider honored. Real alternatives: keep turn caps as an optional per-provider budget dimension (divergent semantics per crew), or forbid them as policy and bound runs with provider-neutral limits only.

## Decision

All run budgets in orbit — job/activity assets, `[qa]` and other config sections, workflow invoke payloads, and any surface that dispatches agent runs — are expressed provider-neutrally: wall-clock timeout as the primary bound, plus neutral resource caps (output bytes, artifact/evidence caps) where needed. Turn caps (`max_turns` or equivalents) must not appear in orbit config schemas, job assets, invoke payloads, or task specs. Provider-specific throttles may exist only inside a single provider adapter's internals as implementation detail, never as configurable cross-provider policy.

## Consequences

- Swapping a crew on a task or workspace never changes budget semantics; runaway runs are bounded identically for every provider.
- Config and job assets stay portable across crews; no per-provider conditionals in dispatch paths.
- Cost: we give up the one cheap fine-grained cap claude offers — a looping claude run now burns wall-clock until the timeout instead of exiting early at a turn count.
- Existing turn-based knobs (e.g. worker `SUPERVISOR_MAX_TURNS` defaults consumed by orbit-driven invokes) are to be retired or demoted to adapter-internal defaults not reachable from orbit policy.