---
title: Global Store Consolidation — Vision
owner: codex
last_updated: 2026-05-23
status: Draft
feature: global-store-consolidation
doc_role: vision
type: design
summary: Future follow-ups for global SQLite runtime-state consolidation.
tags: [global-store-consolidation, storage, sqlite]
paths: ["crates/orbit-store/**", "crates/orbit-core/**"]
related_features: [global-store-consolidation]
related_artifacts: [ORB-00276, ADR-0183]
---

# Global Store Consolidation — Vision

Future work should build on the workspace-scoped tables without making cross-workspace behavior implicit.

## 1. Open Questions

1. Should job-run archive state become an explicit SQLite column before Release N+1 removes legacy directories?
2. Which admin command should own cross-workspace audit and run queries?
3. Should high-volume v2 audit inserts be batched once real-world concurrent write pressure is measured?

## 2. Prior Work

### SQLite Audit

The existing `audit_events` table established `~/.orbit/orbit.db` as the global append-oriented store for CLI invocation audit data.

### Workspace Registry

The task registry established `.orbit/config.yaml` `workspace_id` as the stable workspace identity.

## 3. What May Be Distinctive

The design deliberately keeps file-backed blobs and human-edited friction reports outside the global DB while still consolidating machine-only row data.

## 4. References

- ADR-0183 — accepted consolidation decision.
- ORB-00276 — implementation task.

## Task References

- ORB-00276 — identified Release N+1 cleanup as future work.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
