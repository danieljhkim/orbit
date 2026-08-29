---
title: Task Publication — Design
owner: codex
last_updated: 2026-08-29
last_validated: 2026-08-29
status: Draft
feature: task-publication
doc_role: design
type: design
summary: Proposed contract for publishing validated task snapshots to a dedicated private repository without creating a second mutation authority.
tags: [task-publication, task-artifacts, backup, git, multi-host]
paths: ["crates/orbit-store/src/workflow/task/**", "crates/orbit-registry/**"]
related_features: [task-publication, task-artifacts, remote-access, federated-mcp, host-registry]
related_artifacts: [ORB-11068]
---

# Task Publication — Design

This document specifies a proposed v1 publication protocol. It is not a
description of shipped behavior. V1 publishes one workspace's authority-owned
task snapshots to one dedicated private Git repository for remote durability,
read-only inspection, and explicit recovery. Multi-workspace aggregation,
authority transfer, and multi-writer synchronization remain deferred to
[3_vision.md](./3_vision.md).

## 1. Ownership Boundary

The workspace registry already distinguishes a declared owner from other local
or remote bindings. Task publication uses that existing ownership fact; it does
not elect a leader or infer authority from whichever machine has repository
write permission.

| Concern | Authority |
|---|---|
| Task allocation, lifecycle, relations, and comments | Declared workspace owner |
| Task bundles in `~/.orbit/tasks/workspaces/<workspace-id>/` | Declared workspace owner |
| Publication-repository binding and lineage | Owner machine's Orbit registry |
| Publication branch advancement | Declared workspace owner |
| Repository visibility, collaborators, and retention | Git host and operator |
| Runs, checkpoints, logs, audit, locks, reservations, and indexes | Destination host; never published |
| Source-code branches and pull requests | Existing source-repository workflow |

Only the owner may publish. Consumers do not import a fetched tree into their
live task store automatically. Consequently publication is a derived durability
channel, not replicated task-store leadership.

## 2. Publication Repository and Binding

V1 binds exactly one Orbit workspace to one dedicated publication repository.
The repository contains no source code or other workspaces. This intentionally
trades repository provisioning for the smallest access, ownership, and
compare-and-swap boundary.

The operator provisions an empty private repository on the Git host. Orbit can
verify that the publication repository differs from the source repository and
that its contents match the publication protocol. Generic Git transport cannot
prove provider-side visibility settings, so Orbit reports privacy as
operator-managed unless a provider-specific integration can verify it.

The owner stores a machine-local binding with these logical fields:

```yaml
workspace_id: ws_orbit
source_repository_fingerprint: <portable-source-identity>
publication_remote: <remote-alias-or-url>
publication_branch: refs/heads/main
publication_id: <opaque-publication-lineage-id>
authority_machine_id: hm_example
```

The binding lives with machine-local workspace-registry state, not in a task
bundle or source-controlled `.orbit/config.toml`. It must not contain embedded
credentials, claim tokens, checkout paths, SSH command lines, or
credential-bearing URLs. Authentication uses the operator's existing Git/SSH credential
configuration.

`source_repository_fingerprint` uses the registry's portable remote identity,
not a local path. The fingerprint may need an explicit rebind when the source
repository moves or changes canonical remote; it must never drift silently.

The default publication branch is `refs/heads/main`, but the binding may select
another ordinary branch to match repository policy. Because the repository is
dedicated, no orphan branch is required. First initialization requires an empty
repository and creates the branch's root publication commit. A non-empty
repository without a valid matching publication envelope is refused rather than
adopted or overwritten.

## 3. Tree and Manifest Contract

The proposed publication branch tree is:

```text
orbit-task-publication.yaml
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
publication_id: <opaque-publication-lineage-id>
workspace_id: ws_orbit
source_repository_fingerprint: <portable-source-identity>
authority_machine_id: hm_example
generation: 42
published_at: 2026-08-29T00:00:00Z
task_schema_version: 2
previous_publication: <git-oid-or-null>
attachment_policy: include
task_ids: [ORB-00001, ORB-00002]
omitted_attachments: []
```

The real schema should reuse current canonical workspace, machine, task-schema,
and repository-fingerprint types. It must not contain source checkout paths,
credentials, private repository contents outside the publication tree, or Git
host access tokens.

The manifest's `generation` is monotonic within one publication lineage.
`previous_publication` must equal the parent commit after the root publication.
The Git parent remains the commit graph authority; the duplicated field makes a
detached snapshot self-describing during recovery.

## 4. Snapshot Construction

Publication is an explicit operation that may also be invoked by an
operator-configured routine. It is not part of task-write acknowledgement: a
task update commits to the canonical local store even when the publication
repository is unavailable.

The owner performs these phases:

1. Resolve the exact registered workspace, verify that this machine is its
   declared owner, and load its machine-local publication binding.
2. Open a private temporary clone or Git object cache for the publication
   repository. Verify the repository/branch, publication ID, workspace ID,
   source-repository fingerprint, and authority lineage against the binding.
3. Fetch the configured publication branch and record its observed object ID,
   or confirm that the bound repository is empty during initialization.
4. Read canonical task bundles through `orbit-store` bundle APIs. Validate each
   envelope, JSONL tail, attachment manifest, size, and checksum before staging
   it.
5. Apply the configured attachment policy and write the publication envelope.
6. Build and commit the snapshot inside the private publication-repository
   cache. The source-code repository and its worktree are never checked out,
   switched, staged, or dirtied by publication.
7. Push the new commit as a normal fast-forward update to the configured branch.
8. Record the successful generation and commit ID in owner-local publication
   state. Failed publication does not change the last-success record.

Task bundles have per-bundle durability rather than one workspace-wide read
transaction. A v1 publication is therefore a validated set of individually
consistent bundle observations, not a claim that every task was captured at
the same instant. A later publication converges on newer owner state.

## 5. Compare-and-Swap and Competing Writers

The expected branch tip from phase 3 is load-bearing. The push must fail when
the publication branch moved after it was fetched.

A non-fast-forward rejection means one of the following:

- another machine is publishing as the same workspace authority;
- an operator changed the publication repository manually;
- the local authority binding is stale after an explicit ownership change; or
- the repository/branch belongs to another workspace or publication lineage.

Orbit reports an authority conflict with the observed and current commit IDs.
It does not pull, merge, replay task operations, force-push, select the newest
timestamp, or silently create an alternate branch. Publication resumes only
after the operator resolves the authority or repository-binding discrepancy.

Repository branch protection should reject force-push and deletion when the
Git host supports it. Protection is defense in depth; Orbit still enforces the
fast-forward rule client-side.

## 6. Consumers and Recovery

A consumer clones or fetches the private publication repository into an
Orbit-owned cache. It must not add files or Git state to the source-code
checkout.

### Read-only inspection

An inspector validates the publication envelope and bundle hashes, then renders
tasks directly from the fetched tree or a disposable index. The data is labelled
with publication time, generation, workspace, source-repository fingerprint,
authority, publication ID, and commit ID. It is never presented as live state.

### Disaster recovery

Restore is explicit and fail-closed:

1. Verify the repository and envelope belong to the requested workspace, source
   repository, publication ID, and expected authority lineage.
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

## 7. Privacy and Attached Artifact Policy

A dedicated private repository separates task visibility from source-code
visibility. It does not provide end-to-end encryption, secret classification,
guaranteed erasure, or protection from repository collaborators and Git-host
administrators. Orbit must not advertise stronger privacy than the Git host and
operator configuration can prove.

Core task records and attached files have different exposure and growth risks.
Before first publication, the operator selects one attachment policy:

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

## 8. Failure Semantics

| Failure | Required behavior |
|---|---|
| Publication repository unavailable or Git authentication fails | Leave canonical tasks and last good publication untouched; report publication failure. |
| Publication remote equals the source-code remote | Refuse binding; v1 requires a dedicated repository. |
| Repository is non-empty without a matching publication envelope | Refuse initialization, adoption, and overwrite. |
| Workspace, source fingerprint, publication ID, or authority lineage differs | Stop with a binding conflict; do not import, publish, or rebind implicitly. |
| Bundle, JSONL, manifest, size, or checksum validation fails | Create no commit and push nothing. Name the affected task/path without exposing content. |
| Publication branch moved | Stop with authority conflict; never merge or force. |
| Unsupported newer publication schema | Consumer fails with the supported and observed versions; fetched objects remain untouched. |
| Process exits before push | The remote remains on the last complete commit; clean private temporary state on the next run. |
| Push succeeds but local success recording fails | Re-fetch and reconcile by commit ID before publishing again. |
| Attachment policy rejects bytes | Fail the whole publication unless the operator explicitly selected `omit`. |

## 9. Concerns & Honest Limitations

- V1 provisions one private repository per Orbit workspace. This avoids
  shared-branch serialization and access leakage, but creates repository and credential
  management overhead.
- Git privacy is operator-managed unless a provider integration can verify it.
  A repository accidentally made public exposes every retained publication.
- Publication improves remote durability but is not a complete Orbit backup.
  Audit events, job-run history, claims, reservations, logs, indexes, host
  identity, and runtime configuration remain outside the repository.
- Repository deletion, retention policy, credential loss, or host-side garbage
  collection can remove the only off-host publication unless the operator
  maintains another backup.
- Snapshot history grows monotonically. Git deduplicates unchanged blobs, but
  frequently replaced binary files and JSONL growth can still make fetch and
  storage expensive. V1 performs no automatic history rewrite or compaction.
- Publication freshness is policy, not correctness. An explicit or scheduled
  publisher can lag behind the canonical owner state.
- Per-bundle reads do not create a globally atomic task-store snapshot.
- A read-only consumer cannot continue mutating tasks while the owner is
  offline. That capability requires explicit authority transfer or a new
  multi-writer design.
- Source-repository moves may require an explicit fingerprint rebind even when
  the logical workspace is unchanged.

## Task References

- [ORB-11068] — specified the proposed dedicated private task-publication repository protocol.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
