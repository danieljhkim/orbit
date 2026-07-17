---
title: Host Registry — Decisions
owner: claude
last_updated: 2026-07-16
status: Draft
feature: host-registry
doc_role: decisions
type: design
summary: ADR log for the host-registry feature; no ADRs allocated yet — candidate decisions listed pending acceptance of the design.
tags: [host-registry]
paths: ["crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**"]
related_features: [host-registry, mcp-bridge]
related_artifacts: [ADR-0200, ADR-0205, ADR-0208]
---

# Host Registry — Decisions

ADR log for `host-registry`. Entries are append-only and ordered by ascending global
ID. **Allocate the global `ADR-NNNN` via `orbit.adr.add` before writing the
heading** — never hand-author a four-digit number. The store owns ID, status, owner,
and links; this file is the long-form narrative keyed on that same ID. See
[CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict).

**No ADRs allocated yet.** The feature is Draft ([2_design.md](./2_design.md)); when
the design is accepted, the following candidate decisions each appear to clear the
three-part bar (real alternative, forward constraint, non-trivial cost) and should be
allocated:

1. **The coordination plane is fixed at the main host; workspace ownership is
   per-machine; placement is per-task.** Alternative: per-workspace coordination
   authority (task state scattered across owners, one MCP target per machine).
   Constraint: star topology — exactly one coordination writer, no
   machine-to-machine routes, ownership a declared binding. Cost: a disconnected
   machine cannot author coordination records at all, and hub downtime stalls
   dispatch for every workspace including satellite-owned ones.
2. **Tasks and frictions are hub-only (MCP from every other machine).**
   Alternative: store sync (already rejected once, [ADR-0200] — this extends it to
   writes). Cost: main-host availability becomes a hard dependency for all
   dispatch.
3. **Learnings/ADRs are owner-authored with hub-allocated IDs; canonical MCP reads
   serve hub-owned workspaces; git replicas are explicit, possibly stale reads.**
   Alternative: one-writer-on-hub (satellite prose travels over MCP into a checkout
   the author doesn't hold) or a reservation/finalization protocol. Cost:
   non-owner knowledge authoring is unsupported, and current-state knowledge reads
   don't span owners.
4. **Satellite placement is pull-based (lease poll), never hub-push.**
   Alternative: the hub SSHes to satellites and launches runs. Constraint: the hub
   is a mailbox — it never opens a connection to a satellite; a new `runner`
   capability set exists. Cost: placement latency is bounded by poll cadence, and
   every satellite holds a standing credential against the hub.
5. **Names resolve at binding time; the system persists `machine_id`.**
   Alternative: `host_id` strings everywhere (status quo shape). Constraint:
   renames cannot redirect existing bindings. Cost: the registry carries an
   append-only tombstone alias table forever, and runs/bindings need dual-field
   (id + display name) plumbing.
6. **Requested and actual placement are snapshotted immutably per run.**
   Alternative: task-level `host` field only. Constraint: retries/rescues
   re-resolve the preference; history is never rewritten. Cost: placement fields on
   every run record even in the all-`dk1` common case.
7. **Unpinned committed routines fail closed; scope by location, not git status.**
   Alternative: unpinned = main host, or git-status introspection. Cost: every
   existing committed routine must carry a pin before the lint lands.
8. **Host identity = generated `machine_id` + renameable `host_id`, initialized by
   `orbit init`.** Alternative: hostname-derived free string (status quo). Cost:
   scripted init paths must pass `--host-name` or pre-seed the file.
9. **Registry is main-host inventory in the global store, separate from client
   `mcp.toml` trust policy.** Alternative: merge with `mcp.toml` targets. Cost: two
   places describe hosts, and they can drift.

## Task References

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
