---
type: context
summary: Entry point for Orbit day-2 operations and focused operational runbooks.
tags: [operations, runbooks, day-2]
related_features: [activity-job, auditability, routines]
related_artifacts: [ORB-10014]
---

# Orbit Operations

Day-2 operations for a machine running Orbit are documented as focused runbooks under
[`docs/runbooks/`](./runbooks/). This page remains the stable entry point for existing
links while keeping procedures small enough to retrieve, review, and maintain independently.

Commands in the extracted runbooks were originally verified against `orbit 0.9.2`; update
the affected runbook whenever CLI behavior, state layout, or recovery semantics change.

| Operational need | Runbook |
|---|---|
| Locate authoritative and regenerable state; back up or restore it safely | [Inventory and protect Orbit state](./runbooks/state-and-backup.md) |
| Diagnose, cancel, resume, or replay a stuck job run | [Recover stuck job runs](./runbooks/stuck-job-runs.md) |
| Recover a corrupted SQLite database | [Recover a corrupted database](./runbooks/database-recovery.md) |
| Find, filter, rotate, and retain process logs | [Inspect and retain logs](./runbooks/logging.md) |
| Investigate invocation and pipeline audit history | [Inspect the audit trail](./runbooks/audit-trail.md) |
| Check CLI, dashboard, workspace, and routine-clock health | [Check Orbit health](./runbooks/health-checks.md) |
| Review and apply workspace-layout or store-schema migrations | [Upgrade Orbit safely](./runbooks/upgrades.md) |

Runbook authoring and maintenance rules live in
[`docs/runbooks/CONVENTIONS.md`](./runbooks/CONVENTIONS.md).

These docs are host-agnostic. Host-specific deployment notes—which units are enabled on
which box, ports, sync jobs, and local alerting—belong in the operator's private knowledge
base, not this repository.

Related references: [CONFIG.md](./CONFIG.md) (configuration) ·
[RELEASE.md](./RELEASE.md) / [RELEASING.md](../RELEASING.md) (cutting releases).
