## Context

The I/J wave of the ORB-10246 multi-host plan (ORB-10281 hub run leases, ORB-10282 one-shot spoke runner journal and leased executor, ORB-10283 durable report recovery and fault injection, ORB-10284 minute-clock integration) was designed for unattended pull-based execution: a spoke machine polls the hub, leases runs, survives crashes via a durable local journal, and reports back with strict ack-before-spawn ordering — all with nobody watching.

Since 2026-07-12 the operating model is deliberately the opposite: dispatch is orchestrator-driven with no scheduled ship-sweep. The orchestrator triages, dispatches explicit task ids via workflow_ship, and shepherds every run to done (ship-shepherd / run-rescue). The orchestration layer is already the durability and recovery authority; a spoke-side lease journal would duplicate that one layer down. Additionally, all execution today happens on the machine hosting the hub (worker via bridge, or direct SSH sessions), so no second machine needs to lease work. The lease/journal/poll machinery (I2/I3) is among the hardest, most stateful code in the plan, and none of the four tasks has started.

## Decision

Cancel the runner-lease track: ORB-10281, ORB-10282, ORB-10283, and ORB-10284. Do not build autonomous spoke polling, run leases, runner-only MCP tools, the spoke runner journal, or runner/routine clock composition.

If a second execution machine materializes, reach it with supervised push-style invocation (agent_invoke over the existing SSH-carried MCP link from ORB-10269, which lands independently and is retained), accepting that a crashed remote run is re-dispatched by the shepherd rather than resumed from a local journal.

## Consequences

- The multi-host plan under ORB-10246 shrinks to the landed registry/broker/knowledge waves; remaining H/G units should be re-audited for dependencies they assumed on I/J (placement-aware submission ORB-10280 in particular) before promotion.
- Unattended crash-resumable execution on remote spokes is forgone; recovery for any future remote execution is re-dispatch by the orchestrator, which can duplicate side effects of a partially completed run — acceptable because runs land through PR-gated review or are otherwise idempotent at the task level.
- Cost: if a genuinely unattended multi-machine fleet is ever needed, this track must be re-scoped and re-planned; the cancelled task specs remain in the store as the starting point, but design context will have aged.
- Roll-forward: this decision reverses cheaply — no code was built, no schema shipped, and ORB-10269's transport remains available for either push or pull designs.