## Context

An inline `agent_review` step ran before the PR candidate was committed, pushed, or published and left no independently addressable review Run. Orbit could keep that inline activity and add more output checks, or materialize review only after publication as its own durable child bound to the pushed SHA.

## Decision

For explicit-task PR shipment with review enabled, dispatch exactly one `task_review_pipeline` child after push, PR publication, and task promotion. Snapshot the parent run, task IDs, workspace, explicit review crew, candidate branch, pushed SHA, and PR identity in the child input; require a structured verdict whose reviewed SHA exactly matches that snapshot. Preflight the selected crew and deployed job/activity contract before inserting the implementation run, and reject review outside PR mode.

## Consequences

- Independent review is observable and resumable through normal job-run records and cannot silently inherit the implementation crew.
- `review=false` keeps the implementation-only shipment path, while review-enabled no-diff and local shipments do not invent an unpublished candidate to review.
- Cost: review-enabled shipment adds a child Run and wait boundary after PR publication, increasing latency and requiring source/shipped workflow assets to stay synchronized.