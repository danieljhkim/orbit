## Context
Agents use `orbit.graph.pack` at the start of execution to turn task context selectors into prompt material. Letting that call trigger an unbounded inline graph refresh can make the selector-first workflow appear hung, with no partial selector results or timeout hint.

## Decision
Make `orbit.graph.pack` read the existing graph snapshot by default and return an `auto_refresh.skipped` diagnostic that names the explicit refresh path. Add a `refresh: true` opt-in for callers that accept a potentially slow inline refresh, and add `timeout_ms` so selector projection can return unresolved entries for selectors not reached before the budget expires.

## Consequences
- Context-gathering agents get prompt-visible guidance instead of a silent rebuild when the snapshot is stale.
- Timed-out pack calls can still return the selectors already projected plus unresolved entries for the remainder.
- Cost: default pack reads can be stale until a separate `orbit graph build` or an opt-in refresh updates the branch ref.

---
