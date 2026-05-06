# Task Sync — Decisions

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-06

ADR-style log of non-obvious task-sync decisions. Each entry names the pressure, the choice, and the tradeoff. Entries are append-only and keyed by number; superseded entries are marked, not deleted.

Format for each entry: **Status · Date · Task(s)**, then *Context → Decision → Consequences*. Every ADR names at least one cost. ADRs in this file carry status `Proposed` until the v2 implementation lands; they flip to `Accepted` with the implementing task ID at that point.

---

## ADR-001 — Orphan branch as registry mechanism

**Status:** Proposed · 2026-05 · [T20260505-12]

**Context.** Three plausible designs for "where does the team's task state live?" exist: a coordinator daemon (shared-host), per-host ID suffixes that paper over allocation collisions without a shared store, and a git-based registry. v1 commits to per-engineer deployment ([README](../../../README.md), [POSITIONING](../../POSITIONING.md)), which rules out a coordinator daemon. Per-host suffixes (`T20260504-h7a3-1`) preserve uniqueness but complicate the local commit-message search convention, audit events, and downstream tooling. Knowledge-graph task attribution was removed in [T20260506-11] and is no longer a current consumer. A git-based orphan-branch registry preserves the ID format and uses infrastructure the team already has.

**Decision.** The task registry lives on a git orphan branch at `refs/heads/orbit/tasks` (user-facing name `orbit/tasks`) on the team's shared remote. Every sync-enabled mutation fetches this ref, mutates locally, commits on the branch, and pushes. Atomic git ref update is the coordinator. Reject coordinator daemon: it would violate the v1 per-engineer doctrine and reintroduce the shared-host work that v1 explicitly defers. Reject per-host suffixes: they break ID-format-as-interface across the system.

**Consequences.**
- Sync inherits the team's existing git auth, transport, and ACL.
- The branch is inspectable with standard `git log` and `git diff` tooling.
- The choice of `refs/heads/orbit/tasks` (over `refs/orbit/tasks`) means branch protection, code review tools, and host UIs all recognize the ref without custom config.
- Cost: every sync-enabled mutation requires a network roundtrip. Workspaces that need offline `task add` must keep sync disabled or use the explicit `--offline` escape hatch.

---

## ADR-002 — Operation-aware replay over text-merge or event sourcing

**Status:** Proposed · 2026-05 · [T20260505-12]

**Superseded by:** [T20260506-11] for any premise that Orbit `task_id` must be globally resolvable outside a future sync registry.

**Context.** Standard git text merge fails for task bundles in three concrete ways: status-transition divergence (git keeps both target paths), comment/history list appends (text-merge mangles YAML structure), and same-field edits (humans can't usefully resolve YAML quoting conflicts). Four mechanisms were evaluated against four scenarios in [2_design.md §3.1](./2_design.md): ADD-only sync, operation-aware replay, event sourcing, and no sync. ADD-only ships fast but leaves the partial-coverage mental model that updates don't sync — the operations users care about most. Event sourcing handles every scenario but requires building an event materializer and abandoning YAML-as-canonical. No sync defers the entire problem to v2 shared-host.

**Decision.** v2 sync uses operation-aware replay. The on-branch artifact remains a YAML snapshot; the client treats every push as one of a fixed set of operations (`task.add`, `task.transition`, `task.comment.append`, `task.history.append`, `task.review.append`, `task.field.update`, `task.artifacts.upsert`, `task.delete`) and replays the operation against the new tip when push is rejected. Operations that are convergent by construction (`comment.append`, `history.append`, `review.append`) always retry automatically. Operations that depend on a baseline (`transition`, `field.update`) check the baseline and either retry or surface a structured conflict.

**Consequences.**
- The on-branch artifact stays human-readable and inspectable with `git log`.
- Most concurrent operations replay automatically; only genuine same-field-edit conflicts surface to the user.
- Existing `orbit-store` code keeps owning YAML serialization and layout; the sync coordinator is a new layer above, not a rewrite of the store.
- Cost: when a structured conflict surfaces, the user must run `orbit task sync resolve <task-id>` and choose. The UX must be excellent or the feature becomes friction; we're explicitly trading "magic auto-merge" for "explicit, structured surface."

---

## ADR-003 — Sync scope is task bundles + companion files + artifacts

**Status:** Proposed · 2026-05 · [T20260505-12]

**Context.** Once sync exists, the natural pull is to expand it: the audit DB, scoreboards, job runs, knowledge graph, and SQLite reservation locks all have multi-machine coordination value. Each, however, has different consistency, retention, and merge requirements that don't fit the task-bundle model. Audit is `GlobalOnly` and append-tamper-evident. Scoreboards use counter semantics that don't merge. Locks are TTL-based ephemeral. Job runs are large blob-bearing artifacts. Knowledge graph is content-addressed and branch-scoped.

**Decision.** Task sync covers exactly: `task.yaml`, `plan.md`, `execution-summary.md`, and `artifacts/**` for each task. Locks, audit DB, scoreboards, job runs, and knowledge graph are explicitly out of scope at any version. Each has its own design problem; bundling them into task sync would couple decisions that should remain decoupled.

**Consequences.**
- The feature surface is bounded and shippable.
- Teams that want shared audit, shared scoreboards, or shared locks must wait for separate designs.
- Cost: the per-engineer-deployment doctrine remains true for everything except task bundles. Operators who expect "team sync means everything is shared" will be surprised; documentation must be explicit.

---

## ADR-004 — ID allocation against fetched registry; format unchanged

**Status:** Proposed · 2026-05 · [T20260505-12]

**Superseded by:** [T20260506-11] for any knowledge-graph attribution or cross-machine `task_id` motivation.

**Context.** The `T<YYYYMMDD>-<N>` task ID format is a local interface, not a cross-machine reference. It appears in commit messages (`[T20260421-0528]`), local audit events, and operator scripts that grep `T\d{8}-\d+`. Knowledge-graph node attribution was removed in [T20260506-11], and cross-engineer task references now go through `external_refs`. The current allocator at [layout.rs:102-132](../../../crates/orbit-store/src/file/task_store/layout.rs) scans the local file store and increments — it has no view of other operators' allocations.

**Decision.** ID allocation continues to produce `T<YYYYMMDD>-<N>`; the format is preserved. The allocator gains a "view" abstraction that, when sync is enabled, scans the fetched registry's state directories *plus* any local unpushed tasks before computing the next counter. On push rejection caused by ID collision, the operation is retried via the standard replay path: re-fetch, re-allocate (now seeing the conflicting peer's task), rewrite the bundle locally with the new ID, and retry push. The retry window is safe because allocation happens before any commit message, audit event, or agent dispatch references the ID.

**Consequences.**
- All existing consumers of `T<YYYYMMDD>-<N>` continue to work.
- Allocator becomes view-aware but does not change format or storage.
- Cost: ID allocation requires the registry fetch — `task add` becomes online-only when sync is enabled. This is the largest behavioral change exposed to users.

---

## ADR-005 — Auth piggybacks on git remote credential helper

**Status:** Proposed · 2026-05 · [T20260505-12]

**Context.** Task sync needs to fetch and push to a git remote on every mutation. The team already has an authenticated relationship with that remote (SSH keys, HTTPS tokens, SSO-wrapped credentials, SSH agent, etc.). Building a separate auth surface for Orbit would duplicate that machinery and create a separate credential-rotation problem.

**Decision.** Task sync uses the system git credential helper for fetch/push. There is no Orbit-specific token, no separate ACL, no separate authentication. If the operator can `git push origin main`, they can `orbit task add` against the registry on the same remote. Failures (expired tokens, revoked SSH keys) surface as the same errors `git` itself would produce.

**Consequences.**
- No new auth surface to defend.
- Registry access is bounded by the same ACL that bounds code access.
- Cost: short-lived auth tokens (e.g., SSO-wrapped 8-hour tokens) cause `task add` to fail mid-day at refresh time. Orbit cannot mitigate this without owning auth, which it deliberately does not.

---

## ADR-006 — `git2` (libgit2) over shelling to `git`

**Status:** Proposed · 2026-05 · [T20260505-12]

**Context.** The sync coordinator needs in-process control over fetch, commit, and push: typed errors, programmatic auth callbacks, and the ability to retry without subprocess overhead. Two viable options: the `git2` crate (libgit2 bindings) or shelling to the system `git` binary. Shelling is simpler to reason about — you get exactly what `git` does — but error handling is brittle (stdout parsing) and auth integration with credential helpers requires reimplementing git's helper protocol.

**Decision.** The sync coordinator uses `git2`. Auth callbacks integrate with `git_credential_helper` directly; errors are typed; retries are in-process; the coordinator can hold an open libgit2 handle for the duration of a session.

**Consequences.**
- In-process operation; no subprocess overhead per mutation.
- Auth integrates with system credential helpers via libgit2's existing callbacks.
- Cost: `git2` has a steeper learning curve, occasional ABI churn between releases, and a larger binary footprint than the standalone Orbit binary today. The crate is well-maintained but adds a non-trivial native dependency.

---

## ADR-007 — Sync ships in v2; v1 is per-engineer

**Status:** Proposed · 2026-05 · [T20260505-12]

**Context.** Initial discussion of task sync framed it as a v1 feature — small, opt-in, fits the existing per-engineer doctrine. Subsequent analysis (specifically the conflict-resolution scenarios in [2_design.md §3.1](./2_design.md)) revealed that doing sync correctly requires the operation-aware-replay subsystem in ADR-002, which is meaningful engineering. A half-built sync — for example, ADD-only with no update propagation — produces the wrong mental model: "I can see Bob's task exists but never see him work on it." That's worse for adoption than no sync.

**Decision.** v1 ships per-engineer with no task sync. The default config is `[task.sync] enabled = false` and the sync code path is absent. This design exists in v1 as a docs artifact only. v2 ships sync as an opt-in feature once the operation-aware-replay subsystem and the structured-conflict UX are real.

**Consequences.**
- v1 documentation can confidently say "task sync ships in v2" without weasel wording.
- The conflict-resolution work happens in v2 with adequate scope, not in v1 as a rushed addition.
- The decision to defer is itself documented, so v2 reviewers can challenge it on the same grounds future readers could challenge the v2 mechanism.
- Cost: teams who want shared task visibility *now* don't get it from Orbit; they coordinate via existing git/PR workflows or wait for v2.

---

## ADR-008 — Deletion writes a tombstone, not a hard removal

**Status:** Proposed · 2026-05 · [T20260505-12]

**Context.** `task.delete` could either remove the task directory from the orphan branch or leave it and add a marker indicating deletion. Hard removal is intuitive but creates a footgun: engineer A deletes [T20260504-7]; meanwhile engineer B (offline) made an edit to it; engineer B comes online and pushes; the task reappears as a "new" task with the same ID at the time of B's edit. Tombstones prevent this resurrection by recording the deletion as an explicit operation that subsequent operations honor.

**Decision.** `task.delete` writes a `_tombstones/<task-id>.yaml` entry (with deletion timestamp, deleting agent, and the deleted task's last-known status) and removes the regular task path. Reads ignore tombstoned IDs. Operations against a tombstoned task fail with a clear error referencing the tombstone.

**Consequences.**
- Deleted tasks stay visibly deleted across syncs.
- Audit trail of deletions is preserved on the branch — a deleted task's lifecycle is still inspectable via `git log refs/heads/orbit/tasks -- proposed/<task-id>/`.
- Cost: tombstones accumulate without bound. A future ADR adds a tombstone-pruning policy (e.g., tombstones older than 6 months are hard-removed) once the operational signal exists. Until then, tombstones are forever.

---

## Task References

- [T20260505-12] — Design git-orphan-branch task sync (v2 feature). The task that produced this folder.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
