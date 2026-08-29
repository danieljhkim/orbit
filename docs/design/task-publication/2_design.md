---
title: Task Publication — Design
owner: codex
last_updated: 2026-08-29
last_validated: 2026-08-29
status: Draft
feature: task-publication
doc_role: design
type: design
summary: Proposed contract for publishing validated task snapshots without creating a second mutation authority.
tags: [task-publication, task-artifacts, backup, git, multi-host]
paths: ["crates/orbit-store/src/workflow/task/**", "crates/orbit-registry/**"]
related_features: [task-publication, task-artifacts, remote-access, federated-mcp, host-registry]
related_artifacts: [ORB-11068]
---

# Task Publication — Design

This document specifies a proposed v1 publication protocol. It is not a
description of shipped behavior. V1 publishes authority-owned task snapshots
for remote durability, read-only inspection, and explicit recovery; authority
transfer and multi-writer synchronization remain deferred to
[3_vision.md](./3_vision.md).

## 1. Ownership Boundary

The workspace registry already distinguishes a declared owner from other local
or remote bindings. Task publication uses that existing ownership fact; it does
not elect a leader or infer authority from whichever machine can push.

| Concern | Authority |
|---|---|
| Task allocation, lifecycle, relations, and comments | Declared workspace owner |
| Task bundles in `~/.orbit/tasks/workspaces/<workspace-id>/` | Declared workspace owner |
| Publication ref advancement | Declared workspace owner |
| Runs, checkpoints, logs, audit, locks, reservations, and indexes | Destination host; never published |
| Code branches and pull requests | Existing repository workflow |

Only the owner may publish. Consumers do not import a fetched tree into their
live task store automatically. Consequently publication is a derived durability
channel, not replicated task-store leadership.

## 2. Ref and Tree Contract

The remote ref is:

```text
refs/heads/orbit/tasks
```

The first commit is an orphan root. Later commits form a normal first-parent
history on that ref. The ref must never be merged into `main`, `agent-main`, or
a feature branch.

The proposed tree is:

```text
orbit-task-publication.yaml
workspaces/
  <workspace-id>/
    tasks/
      <task-id>/
        task.yaml
        description.md
        acceptance.md
        plan.md
        execution-summary.md
        events.jsonl
        comments.jsonl
        artifacts/
          manifest.yaml
          files/**
```

`orbit-task-publication.yaml` is a publication envelope, not workspace runtime
configuration:

```yaml
format_version: 1
workspace_id: ws_orbit
authority_machine_id: hm_example
generation: 42
published_at: 2026-08-29T00:00:00Z
task_schema_version: 2
previous_publication: <git-oid-or-null>
attachment_policy: include
task_ids: [ORB-00001, ORB-00002]
omitted_attachments: []
```

The real schema should use the current canonical workspace and machine-ID
types. It must not contain checkout paths, credentials, SSH host aliases, claim
tokens, or repository-local secrets.

The manifest's `generation` is monotonic within one authority lineage.
`previous_publication` must equal the parent commit after the root publication.
The Git parent remains the commit graph authority; the duplicated field makes a
detached snapshot self-describing during recovery.

## 3. Snapshot Construction

Publication is an explicit operation that may also be invoked by an
operator-configured routine. It is not part of task-write acknowledgement: a task update
commits to the canonical local store even when the network or Git remote is
unavailable.

The owner performs these phases:

1. Resolve the exact registered workspace and verify that this machine is its
   declared owner.
2. Fetch `refs/heads/orbit/tasks` and record its observed object ID, or record
   that the ref does not exist for first publication.
3. Read canonical task bundles through `orbit-store` bundle APIs. Validate each
   envelope, JSONL tail, attachment manifest, size, and checksum before staging
   it.
4. Apply the configured attachment policy and write the publication envelope.
5. Build an isolated Git tree and commit whose parent is the observed remote
   tip. The implementation uses a private temporary index/object-building
   adapter or equivalent library API; it never checks out the orphan branch in
   the user's code worktree.
6. Push the new commit as a normal fast-forward update to
   `refs/heads/orbit/tasks`.
7. Record the successful generation and commit ID in owner-local publication
   state. Failed publication does not change the last-success record.

Task bundles have per-bundle durability rather than one workspace-wide read
transaction. A v1 publication is therefore a validated set of individually
consistent bundle observations, not a claim that every task was captured at
the same instant. A later publication converges on newer owner state.

## 4. Compare-and-Swap and Competing Writers

The expected remote tip from phase 2 is load-bearing. The push must fail when
the ref moved after it was fetched.

A non-fast-forward rejection means one of the following:

- another machine is publishing as the same workspace authority;
- an operator changed the ref manually;
- the local authority record is stale after an explicit ownership change; or
- the remote ref belongs to another workspace or publication lineage.

Orbit reports an authority conflict with the observed and current commit IDs.
It does not pull, merge, replay task operations, force-push, select the newest
timestamp, or silently seed a second ref. Publication resumes only after the
operator resolves the ownership or remote-ref discrepancy.

Repository branch protection should reject force-push and deletion when the
Git host supports it. Protection is defense in depth; Orbit still enforces the
fast-forward rule client-side.

## 5. Consumers and Recovery

Fetching uses an Orbit-private local ref or object cache. It must not create a
local branch that looks like a code branch and must not modify the current
checkout.

### Read-only inspection

An inspector validates the publication envelope and bundle hashes, then renders
tasks directly from the fetched tree or a disposable cache. The data is labelled
with publication time, generation, authority, and commit ID. It is never
presented as live state.

### Disaster recovery

Restore is explicit and fail-closed:

1. Verify the ref belongs to the requested workspace and expected authority
   lineage.
2. Validate publication and task schema compatibility, every included bundle,
   and every included attachment checksum.
3. Require an empty destination or a deliberate operator-selected recovery
   mode. Any non-identical live task-ID collision aborts the restore.
4. Restore canonical bundles, rebuild registry indexes and checkout
   projections, and advance the local allocator beyond the restored IDs.
5. Report every omitted attachment; an incomplete publication cannot produce a
   "complete backup restored" result.

The existing task importer's `renumber` policy remains useful for migration
between unrelated registries. It is not valid for same-authority recovery,
because changing task IDs would break commit, relation, and audit references.

## 6. Attached Artifact Policy

Core task records and attached files have different exposure and growth risks.
Before first publication, the operator selects one policy:

| Policy | Behavior |
|---|---|
| `fail` | Refuse publication when any attached artifact exists. Safest default until the operator makes an exposure choice. |
| `include` | Publish admitted attachment bytes after path, type, per-file, total-size, deny-pattern, and sensitivity checks. |
| `omit` | Publish the core task record without its attachment manifest/files and record every omitted logical path, size, and hash in the publication envelope. The snapshot is explicitly incomplete. |

The publication layer enforces its own limits. The tool-level artifact upload
limit is not sufficient because imports and future writers may have different
limits. At minimum policy includes:

- maximum bytes per file and per publication;
- canonical relative-path validation;
- deny patterns for credentials, environment dumps, private keys, and known
  secret-bearing outputs;
- an explicit response to a sensitivity scanner failure or unavailable scanner;
- deterministic reporting of included and omitted bytes.

`omit` must build a valid publication projection rather than leave a manifest
pointing at absent blobs. Recovery preserves and reports the omission ledger.

Git history is durable. Deleting an attachment in a later snapshot does not
remove its older blob. If a secret is published, Orbit reports that history
rewrite and credential rotation are operator incidents; it must not claim that
deleting the current tree erased the data.

## 7. Failure Semantics

| Failure | Required behavior |
|---|---|
| Remote unavailable or Git authentication fails | Leave canonical tasks and last good publication untouched; report publication failure. |
| Bundle, JSONL, manifest, size, or checksum validation fails | Create no commit and push nothing. Name the affected task/path without exposing content. |
| Remote ref moved | Stop with authority conflict; never merge or force. |
| First publication finds an existing ref | Validate its publication envelope; refuse implicit adoption or replacement. |
| Unsupported newer publication schema | Consumer fails with the supported and observed versions; fetched objects remain untouched. |
| Process exits before push | The remote remains on the last complete commit; clean private temporary state on the next run. |
| Push succeeds but local success recording fails | Re-fetch and reconcile by commit ID before publishing again. |
| Attachment policy rejects bytes | Fail the whole publication unless the operator explicitly selected `omit`. |

## 8. Concerns & Honest Limitations

- Publication improves remote durability but is not a complete Orbit backup.
  Audit events, job-run history, claims, reservations, logs, indexes, host
  identity, and runtime configuration remain outside the ref.
- The Git remote becomes part of disaster recovery. Repository deletion,
  retention policy, credential loss, or host-side garbage collection can remove
  the only off-host publication unless the operator maintains another backup.
- Snapshot history grows monotonically. Git deduplicates unchanged blobs, but
  frequently replaced binary files and JSONL growth can still make fetch and
  storage expensive. V1 performs no automatic history rewrite or compaction.
- Publication freshness is policy, not correctness. An explicit or scheduled
  publisher can lag behind the canonical owner state.
- Per-bundle reads do not create a globally atomic task-store snapshot.
- A read-only consumer cannot continue mutating tasks while the owner is
  offline. That capability requires explicit authority transfer or a new
  multi-writer design.
- Standard Git authorization may allow more repository writers than Orbit
  workspace authorities. Client-side checks and branch protection reduce but do
  not eliminate malicious or out-of-band ref mutation.

## Task References

- [ORB-11068] — specified the proposed authority-owned task publication protocol.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
