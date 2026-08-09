## Context

**Withdrawn 2026-08-08, never accepted.** This ADR proposed moving review-crew selection onto the task. Daniel subsequently decided to remove independent review from Orbit entirely (ORB-10628), so there is no review routing left to own. The record is kept because the reasoning about mechanism-versus-policy outlived the feature: Orbit ships mechanism, and a rule about who may review whom belongs to the operator. Do not implement this. ORB-10623, ORB-10624, and ORB-10625 were rejected alongside it.

The original context follows.

The review crew is chosen by a ship-time flag, and preflight rejects any review crew that matches a task's implementation crew — by crew name, or by identical model, provider, and backend. That encodes one operator's adversarial-review policy in Orbit itself, and it makes "who reviewed this" a property of an invocation rather than of the task, so review attribution cannot be recovered from durable state. A single ship also carries a single review crew across every task in it. The alternative was to keep ship-level selection and tighten the built-in independence rule to compare provider families, which would have deepened the policy Orbit encodes rather than removing it.

## Decision

Superseded by removal. The proposal was: an optional `reviewer` field on the task names the crew that reviews it and is the source of truth; the ship-time flag applies only to tasks that declare none, and the configured system crew is the final fallback. Orbit would validate only that a named reviewer crew resolves to a usable model, provider, and backend. Review dispatch would fan out one child per reviewed task.

## Consequences

- None. The feature this decision governs is being deleted rather than reworked.
- The mechanism-versus-policy reasoning survives independently: ADR-0330 keeps the rule that routing is expressed through configuration rather than code, and the operator's cross-provider review convention now lives in the operator's own instructions rather than in Orbit.
- Cost: the review attribution problem this ADR was written to solve is not solved, it is removed. If independent review is ever rebuilt on the simplified pipeline, per-task reviewer ownership and the absence of a built-in independence check should be revisited as the starting design rather than rediscovered.