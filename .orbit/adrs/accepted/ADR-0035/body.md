## Context
Policy denials and friction submissions reached canonical stores or return paths, but operators tailing the live feed could miss them.

## Decision
Emit structured `tracing::warn!` projections beside canonical side effects for filesystem denials, proc-spawn denials, and friction task submissions.

## Consequences
- Dashboards can watch `orbit.policy.deny` and `orbit.friction.reported` without querying canonical stores.
- Cost: the tracing feed is lossy and filterable, so missing live events cannot prove the canonical store has no matching record.
