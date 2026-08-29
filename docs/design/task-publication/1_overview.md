---
title: Task Publication — Overview
owner: codex
last_updated: 2026-08-29
last_validated: 2026-08-29
status: Draft
feature: task-publication
doc_role: overview
type: design
summary: Publish authority-owned task-bundle snapshots to an isolated Git branch for remote durability and read-only recovery.
tags: [task-publication, task-artifacts, backup, git, multi-host]
paths: ["crates/orbit-store/src/workflow/task/**", "crates/orbit-registry/**"]
related_features: [task-publication, task-artifacts, remote-access, federated-mcp, host-registry]
related_artifacts: [ORB-11068]
---

# Task Publication — Overview

Task publication is a proposed one-way durability surface: the machine that
owns a workspace publishes validated task-bundle snapshots to an orphan Git
branch, while other machines may inspect or explicitly restore those snapshots
without becoming competing task authorities.

Nothing in this folder is implemented yet. The design deliberately stops short
of the retired multi-writer task-sync proposal.

## 1. Motivation

Orbit's canonical task bundles live under
`~/.orbit/tasks/workspaces/<workspace-id>/`. That keeps task mutation and local
execution state under one machine's authority, but it also makes the bundle
corpus dependent on host backup unless the operator exports it deliberately.

Existing capabilities solve adjacent problems:

- [Remote Access](../remote-access/1_overview.md) and
  [Federated MCP](../federated-mcp/1_overview.md) route live reads and mutations
  to the owning host. They require that host to remain reachable.
- `orbit task export` and `orbit task import` move validated archives between
  registries. They are explicit migration tools rather than a repository-scoped
  remote backup channel.
- The retired [Task Sync](../_archive/task-sync/1_overview.md) design made every
  mutation network-dependent and required operation-aware multi-writer conflict
  resolution.

Task publication fills the narrower remaining gap: keep a remote,
repository-scoped, inspectable copy of task intent and selected attachments without
changing which machine owns task allocation, lifecycle, or execution.

## 2. Core Concepts

### Publication authority

The workspace's declared owner is the only machine allowed to advance its
publication. A successful publication does not transfer task authority.

### Publication ref

`refs/heads/orbit/tasks` is an orphan Git branch with no ancestry shared with
code branches. Its commits are immutable snapshots of published task bundles,
not code changes and not a second writable task store.

### Publication snapshot

A validated tree containing one workspace manifest, task bundles, and the
attached artifact bytes admitted by the configured publication policy. Each
commit records the previous publication as its parent after the first orphan
commit.

### Consumer

A machine that fetches the publication ref for inspection or disaster recovery.
Fetching does not bind the snapshot as a live writable workspace and does not
authorize local task mutation.

### Artifact publication policy

An explicit choice governing attached bytes under `artifacts/files/`. Orbit
never silently sends every local attachment to the code remote and never calls
an attachment-omitting snapshot a complete backup.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Proposed ownership and publication protocol | [2_design.md](./2_design.md) | [ORB-11068] |
| Evolution gates and deferred synchronization | [3_vision.md](./3_vision.md) | [ORB-11068] |
| Standing authority and safety rules | [4_decisions.md](./4_decisions.md) | [ORB-11068] |
| Canonical task-bundle format | [Task Artifacts](../task-artifacts/2_design.md) | — |
| Current live cross-machine access | [Remote Access](../remote-access/1_overview.md) | — |
| Current host-qualified MCP routing | [Federated MCP](../federated-mcp/1_overview.md) | — |
| Portable archive and restore primitives | [`orbit-store` task workflow](../../../crates/orbit-store/src/workflow/task/mod.rs) | — |
| Retired active-active proposal | [Task Sync](../_archive/task-sync/1_overview.md) | — |

## Task References

- [ORB-11068] — designed authority-owned task publication through an orphan Git branch.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
