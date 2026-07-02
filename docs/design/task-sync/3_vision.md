---
summary: "Task Sync — Vision"
type: design
title: "Task Sync — Vision"
owner: claude
last_updated: 2026-07-02
status: Draft
feature: task-sync
doc_role: vision
tags: ["task-sync"]
---

# Task Sync — Vision

This document captures the open questions task sync deliberately leaves unanswered, the prior work the design builds on or rejects, what the design contributes that's specific to Orbit, and external references for readers who want to dig deeper. The questions in §1 are the most likely sources of post-sync design pressure; the prior work in §2 explains why the design landed where it did.

---

## 1. Open Questions

### 1.1 Orphan-branch compaction

Every task lifecycle event is a commit on `refs/heads/orbit/tasks`. A team running 50 tasks/week through 5–10 status transitions each will produce 250–500 commits/week. After a year, the branch has tens of thousands of commits — cloning it gets slow, `git log` becomes useless without filtering, and the branch carries history for archived tasks no one wants to load.

The initial sync design ships without compaction. Three candidate strategies for a follow-up:

- **Snapshot + prune.** Periodically write a "snapshot" commit that orphan-recreates the branch with current-state-only and discards prior history. Loses the per-task audit trail on the branch but preserves the YAML history field inside each task.
- **Sparse fetch by age.** `task sync pull` defaults to the last N months; older tasks load on demand. Doesn't shrink the branch, just the local checkout.
- **Per-status retention.** Archived and rejected tasks beyond an age threshold are tombstoned and the bundle removed. Loses recoverability of old terminal-state tasks.

Decision deferred until operational pain manifests on a real team's branch.

### 1.2 Should sync fold into shared-host?

Task sync is opt-in and orthogonal to the shared-host coordinator. If shared-host ships, there are two coherent end states:

- **Sync remains.** Teams can choose: orphan-branch sync (no server, slower, partially manual) or shared-host (server, fast, central). Both ship.
- **Sync is deprecated.** Shared-host becomes the team-coordination story; orphan-branch sync is removed because it duplicates functionality with worse properties.

The honest answer depends on demand. Some teams won't run a shared service. Some teams will. The design does not pre-commit; it builds task sync as a standalone feature that can survive or be retired without restructuring the rest of Orbit.

A third, lighter point in this space now ships and narrows the question. `orbit web connect` plus the global multi-workspace dashboard ([ORB-00029], [ORB-00030]) give live, browser-based visibility into a machine's workspaces — a peer's over an SSH tunnel, or all of your own at once — with no server, no sync branch, and no shared store. It does not fold sync into shared-host so much as carve the gap in two: the pure *viewing* half of "see the team's tasks" is answerable today with an SSH tunnel, leaving durable, offline-capable, writable shared state as the part only task sync (or a shared host) provides. Watching how much of the perceived need `web connect` absorbs is itself a useful demand signal before committing to build sync. See [4_decisions.md ADR-009](./4_decisions.md).

### 1.3 Soft-claim semantics

The recommended design proposes `assigned_to` as an advisory field, not a lock. Open questions:

- Does `assigned_to` expire? If engineer A claims `ORB-00042` and is on vacation, engineer B should be able to take over without an admin intervention.
- Is there a "soft-stealing" warning when engineer B updates a task assigned to engineer A?
- Is the field per-task or per-status (assignee for backlog vs. assignee for in_progress could be different)?

These are UX questions that don't block the transport design but matter for whether the feature is pleasant to use.

### 1.4 CI-friendly task creation

A team that runs `task add` from CI (e.g., as part of an automated triage pipeline) hits the online-only mutation requirement. Short-lived CI tokens or rate-limited git operations may not handle the fetch+push pattern reliably.

Options for a follow-up:

- **CI-side coordinator.** A long-running CI service that batches task adds with a single fetch.
- **Async submission queue.** `task add --offline` writes to a local queue; a separate `task sync push --queue` flushes it later.
- **Explicit CI mode.** Detect CI environment and either disable sync or use a different transport.

### 1.5 Multi-remote and private tasks

A team may want some tasks in a public-facing remote and others in a private one (security incidents, internal tooling). The initial design assumes a single registry remote per workspace. Open questions:

- Should the config support multiple `[task.sync.<name>]` blocks, one per remote?
- Should tasks carry a `visibility` field that routes them to the appropriate remote?
- Or is this just outside Orbit's responsibility — the team should run two workspaces?

The single-remote default keeps the first release simple; multi-remote is reserved for a follow-up.

### 1.6 Whether to ever go event-sourced

Operation-aware replay is the current choice precisely because it's incremental on the existing bundle-snapshot model. If conflict surfaces multiply — particularly the same-field-edit case in [2_design.md §3.1(d)](./2_design.md) becoming common — the event-sourced model becomes more attractive. The migration path from snapshot-canonical to event-canonical is non-trivial; the design preserves the option but does not preemptively pay the cost.

### 1.7 Branch-protection enforcement at the Orbit layer

The design assumes the team's git host supports branch protection on `orbit/tasks` and the team configures it. Hosts without branch protection (or teams who forget) risk a maintainer accidentally merging the orphan branch into `main`. Should Orbit detect and warn? Refuse to operate? Push a special tag?

The honest answer is "this is a host concern, not Orbit's." But it's worth flagging.

### 1.8 Tombstone retention

`task.delete` writes a tombstone, not a hard removal. Tombstones accumulate. A pruning policy (e.g., tombstones older than 6 months get hard-removed) is a useful follow-up but not blocking for the first sync release.

---

## 2. Prior Work

### 2.1 Distributed-state-on-git

Several projects use git itself as a coordination substrate for non-code data:

- **`gh-pages`.** The original orphan-branch pattern. Documents are stored on a separate orphan branch in the same repo, deployed via `git push`. The pattern is operationally well-understood, but `gh-pages` is read-only on push (no concurrent-writer story) — task sync's contribution is the operation-aware replay layer.
- **`git-notes`.** Annotates commits with metadata in a parallel ref namespace. Designed for CI metadata and code review. Conflict semantics are similar to ours (refs/notes are pushable, fetchable, and subject to non-fast-forward rejection), but git-notes assumes additive use; concurrent edits to the same note text fail to merge.
- **`git-bug`.** A bug tracker that uses git refs as the storage. Closer in spirit to task sync than anything else. Uses an event-log model under separate refs (`refs/bugs/`). The design considered this approach explicitly and rejected event sourcing for the first sync release — see [4_decisions.md ADR-002](./4_decisions.md). git-bug is the strongest precedent that "tasks on git refs" is a coherent model.
- **`jj op log`.** The Jujutsu version-control system maintains an operations log as a parallel ref. Operations are first-class and replayable. Architecturally similar to event sourcing but at the VCS layer. Inspirational for the operation-aware replay model in [2_design.md §3.2](./2_design.md).
- **`pijul`.** Patch-based VCS that side-steps the merge problem at a fundamental level. Out of scope for emulation but cited as an example of "git's three-way merge is not the only option."

### 2.2 Event sourcing and CQRS

Task sync's recommended mechanism (operation-aware replay) is event-sourcing-adjacent without the full event-sourcing tax:

- **CQRS / event sourcing.** Conventional ES stores events as the source of truth and materializes views by replay. Task sync stores YAML snapshots as the source of truth and replays *operations* on push reject — a hybrid. The advantage is that the on-branch artifact is human-readable; the cost is that some scenarios (concurrent same-field edit) cannot resolve automatically.
- **Operational transformation (OT).** The Google Docs / Etherpad lineage. Continuous low-latency editing with automatic conflict resolution at character granularity. Not applicable to task semantics — task fields aren't free text being edited continuously by multiple users in the same second.
- **CRDTs (Conflict-free Replicated Data Types).** Algebraically guaranteed convergence. Strong fit for comment append (grow-only set) but overkill for the rest of task state. The design uses CRDT-shaped reasoning for `task.comment.append` and `task.history.append` (always-converging) without committing the full CRDT machinery for the rest.

### 2.3 Issue-tracker sync

Various tools sync GitHub issues, Jira tickets, Linear tasks, etc., across boundaries:

- **GitHub Issues + GitHub Actions.** Webhooks fan out, but conflict resolution is via the host UI (last-write-wins with manual merge). The host is the central coordinator.
- **Jira + Linear sync tools.** API-driven; rely on each system's own conflict semantics. None applicable to a self-hosted, no-coordinator design.
- **Notion / Coda task databases.** Cloud-only. Out of scope.

The closest precedent is `git-bug`. Everything else assumes a server.

---

## 3. What May Be Distinctive

Three properties distinguish task sync from the prior art:

### 3.1 Direct bundle-shape reuse

The on-branch tree mirrors the canonical v2 bundle shape under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/`. There is a projection step back into `.orbit/tasks/<task-id>` for local browsing, but no second task schema: `git log refs/heads/orbit/tasks -- workspaces/orbit-a3f9c2/ORB-00042/task.yaml` returns the metadata lifecycle of `ORB-00042` without an opaque store. `git-bug` and `jj op log` use opaque structures readable only by their own tooling; task sync uses YAML, Markdown, and JSONL at standard paths.

### 3.2 Operation-aware replay tailored to task semantics

The replay rules in [2_design.md §3.2](./2_design.md) are not generic — they encode Orbit's specific knowledge that comment-append is convergent, status-transition is conditional on source state, and field-update needs a baseline check. Generic three-way merge would handle none of these correctly. Generic event sourcing would handle them all but at high architectural cost.

### 3.3 No new auth surface

Task sync inherits the team's existing git auth posture. Whatever the team uses for `git push origin main` works for `task add`. There is no separate token, separate ACL list, separate permission system. This matches the non-negotiable: "self-hostable, no cloud dependency." It also bounds the security surface — a compromise of the registry ref is a compromise of the git remote, not a separate failure mode.

---

## 4. References

### 4.1 Orbit-internal

- [docs/POSITIONING.md](../../POSITIONING.md) — the per-engineer-deployment doctrine that motivates this design.
- [README.md](../../../README.md) — `Direction of travel` section names shared-host as the team-coordination direction; task sync slots underneath that as a git-native mechanism.
- [docs/design/CONVENTIONS.md](../CONVENTIONS.md) — folder layout, frontmatter, ADR template.
- [docs/design/_archive/knowledge-graph/](../_archive/knowledge-graph/) — content-addressed branch-scoped storage; relevant precedent for "Orbit data lives in branch-aware structures."
- [docs/design/auditability/](../auditability/) — relevant for understanding why audit DB is explicitly out-of-scope for task sync.
- [docs/design/user-interface/4_decisions.md](../user-interface/4_decisions.md) — ADR-00030 (global multi-workspace dashboard); the shipped live-viewing path §1.2 positions against sync.

### 4.2 External

- **git-bug** — `https://github.com/MichaelMure/git-bug`. Closest precedent for tasks-on-git-refs.
- **Jujutsu (jj) operations log** — `https://github.com/martinvonz/jj`. Operation-as-first-class-citizen at the VCS layer.
- **Pijul** — `https://pijul.org`. Patch-theoretic VCS as a counterpoint to git's three-way merge.
- **Martin Kleppmann, *Designing Data-Intensive Applications*** — chapters on event sourcing, CQRS, and conflict-free replication. The conceptual frame for [2_design.md §3](./2_design.md).
- **Atlassian Bitbucket "Server Refs" docs** — branch-protection conventions for non-`refs/heads/` namespaces; consulted when evaluating the `refs/orbit/tasks` alternative rejected in [4_decisions.md ADR-001](./4_decisions.md).

---

## Task References

- [T20260505-12] — Original git-orphan-branch task sync proposal. Historical reference; the design now targets the `ORB-*` task-artifact shape.
- [ORB-00029] — `orbit web connect <ssh-host>`: SSH-tunnel dashboard viewing; the shipped viewing path §1.2 weighs against sync.
- [ORB-00030] — Global, multi-workspace dashboard (`orbit web serve --global`).

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
