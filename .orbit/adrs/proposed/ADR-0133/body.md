## Context
Once sync exists, the natural pull is to expand it: the audit DB, scoreboards, job runs, knowledge graph, and SQLite reservation locks all have multi-machine coordination value. Each, however, has different consistency, retention, and merge requirements that don't fit the task-bundle model. Audit is `GlobalOnly` and append-tamper-evident. Scoreboards use counter semantics that don't merge. Locks are TTL-based ephemeral. Job runs are large blob-bearing artifacts. Knowledge graph is content-addressed and branch-scoped.

## Decision
Task sync covers exactly: `task.yaml`, `plan.md`, `execution-summary.md`, and `artifacts/**` for each task. Locks, audit DB, scoreboards, job runs, and knowledge graph are explicitly out of scope at any version. Each has its own design problem; bundling them into task sync would couple decisions that should remain decoupled.

## Consequences
- The feature surface is bounded and shippable.
- Teams that want shared audit, shared scoreboards, or shared locks must wait for separate designs.
- Cost: the per-engineer-deployment doctrine remains true for everything except task bundles. Operators who expect "team sync means everything is shared" will be surprised; documentation must be explicit.

---
