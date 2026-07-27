## Context

Every periodic need in Orbit was previously bespoke code, and each future recurring chore meant another hardcoded routine. The marginal cost of a periodic chore was therefore a code change, review, and release.

## Decision

Introduce auto-tasks as git-versioned YAML definitions under `.orbit/auto_tasks/<name>.yaml`, with cron/interval schedules, host-local cursors, task templates, dedupe, and provenance. One generic deterministic scheduler activity wrapped in a job and fired by the seeded routine processes every definition. Definitions parse fail-closed, catch-up collapses, and CRUD is available through CLI and MCP. Templates remain provider-neutral per ADR-0217.

## Consequences

- Periodic work becomes data; QA sweep is the first checked-in definition.
- Definitions are workspace-scoped and scheduler fires remain observable through routine health.
- Host-local cursor state avoids churn in git-versioned definitions.
- Cost: a second file-backed record convention exists alongside the SQLite-indexed knowledge records, and auto-task definitions are not full-text indexed.