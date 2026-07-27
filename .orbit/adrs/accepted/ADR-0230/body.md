## Context
A hub-push executor model would require outbound spoke routing and makes retries obscure the placement actually selected and leased.
## Decision
Spokes poll the hub for placed runs; requested and actual placement are immutable, pre-start loss returns a run for redelivery, and post-start uncertainty requires explicit recovery.
## Consequences
- The hub is a mailbox and never opens a route to a spoke.
- Cost: pickup latency follows poll cadence and an interrupted started run requires operator/shepherd recovery rather than silent reassignment.