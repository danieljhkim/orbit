---
title: Resident Orchestrator — Design
owner: codex, grok, claude
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Draft
feature: resident-orchestrator
doc_role: design
type: design
summary: epic_pipeline owns one worktree and one branch per epic, lands children into it sequentially, finishes the work with an in-worktree agent, and delivers once; workspace_auto_pipeline drains the backlog for a caller-supplied window. Session log is the memory between fires.
tags: [resident-orchestrator, epic, jobs, session-log]
paths: [".orbit/resources/jobs/**", "crates/orbit-core/assets/jobs/**", "crates/orbit-core/assets/activities/**"]
related_features: [resident-orchestrator, activity-job]
related_artifacts: [ORB-10332, ORB-10775, ORB-10776, ORB-10779, ORB-10788, ORB-10815, ORB-10816, ORB-10817, ORB-10818, ORB-10819]
---

# Resident Orchestrator — Design

> **Status: Draft, landing incrementally.** This is the v2 contract ([ORB-10815]).
> [§10](#10-what-v1-did-and-why-it-changed) records the v1 shape and why each piece changed. It
> still does not add a resident server, an Orbit routine, conversation resume, or a comment-typed
> decision protocol.
>
> | Section | State |
> |---|---|
> | [§1 Epic worktree and sequential child drain](#1-epic-worktree-and-sequential-child-drain) | **live** ([ORB-10816]) |
> | [§2 Epic agent](#2-epic-agent-epic_orchestrator) | planned ([ORB-10817]) — the shipped activity is still the v1 dispatcher |
> | [§3 `epic_pipeline` job](#3-epic_pipeline-job) | partly live — §1's phases ship; the agent step, completion gate, and delivery are [ORB-10818] |
> | [§4 Workspace drain](#4-workspace-drain-workspace_auto_pipeline) | planned ([ORB-10819]) — the epic reservation is live, but `decision: hold` and the one-action tick still ship |
> | [§5](#5-unresolved-work-scan)–[§8](#8-authority-and-completion) | live |

## 1. Epic worktree and sequential child drain

> **Live** since [ORB-10816].

An epic is one body of work, so it produces one branch that a human reviews once ([An epic owns one worktree and one branch](./4_decisions.md#an-epic-owns-one-worktree-and-one-branch), [ORB-10816]).

`epic_pipeline` opens a **stable** worktree for the epic root through `worktree_setup`, passing an
explicit `run_id: epic-<ORB-id>` and `branch_prefix: epic`. `WorktreeIdentity` derives the
directory from those two inputs, so the same epic resolves to the same worktree and branch across
runs and reattaches rather than forking a second one. The root moves to `in-progress` and stays
there for the life of the run, which is also what keeps `worktree_gc` from reclaiming the directory.

The `list_epic_descendants` deterministic action returns the root's non-terminal descendants in
dependency then priority/age order. The job's `drain` loop lands them **one at a time**, each
through an `invoke_and_wait` on `task_local_pipeline` followed by a `pipeline_success_guard` so a
failed child stops the drain instead of silently skipping:

| Input | Value | Why |
|---|---|---|
| `base_branch` | the epic branch | children stack onto the epic, not the workspace base |
| `base_sync` | `local` | the epic branch has no remote counterpart |
| `auto_push` | `false` | only delivery publishes |
| `landing_branch` | the real workspace base | ORB-10644 obsolescence gate fails closed once the epic lands |
| `terminal_status` | `done` | the child is finished when it merges into the epic branch |

A child reaches **`done`** on merge into the epic branch, not `review`. `task_local_pipeline`
defaults `terminal_status` to `review`, so only an epic drain opts into `done`; the epic root is
then the single review artifact, and N children in `review` for one epic is the fragmentation this
design removes.

Sequential is a requirement, not a simplification. `merge_with_rebase_retry` lands a child branch
with `git merge --ff-only` plus at most two rebases against the moved base. Sequential children
rebase onto the advanced epic branch and fast-forward cleanly; parallel children race that
fast-forward and exhaust the retry budget. Siblings under one epic also overlap files by
construction, so the gate's reserve-and-wait loop buys nothing the epic's own ordering does not
already provide.

An epic with **no** children is normal and skips this phase entirely.

## 2. Epic agent (`epic_orchestrator`)

> **Planned** ([ORB-10817]). The shipped `epic_orchestrator` is still the v1 no-code-change
> workspace-drain dispatcher: its description forbids editing the repository, and `epic_pipeline`
> does not invoke it. Everything below is the target contract.

`epic_orchestrator` is an `agent_loop` / `backend: cli` activity that runs **inside the epic
worktree**, after the children have merged ([The epic agent works in the worktree instead of dispatching](./4_decisions.md#the-epic-agent-works-in-the-worktree-instead-of-dispatching), [ORB-10817]).

- **Tools:** worktree-scoped writes and process execution on the same footing as `agent_implement`
  (`fs.read`, `fs.delete`, `proc.spawn` over the repo's allowed programs), plus `orbit.task.*`,
  `orbit.search`, and `orbit.session_log.*`.
- **Mandate:** validate the merged result of the children, resolve integration gaps, finish what
  the children did not cover, and bring the epic to the state its acceptance criteria describe.
  Subagents are a provider capability — the CLI runner spawns the provider CLI directly — not new
  Orbit machinery.
- **May:** author further children when decomposition is genuinely warranted. The job loops back to
  §1 when it does.
- **Must not:** write outside the epic worktree, merge PRs reserved for the human merge authority,
  edit a second workspace, or treat silence as new product authority.
- **Bound:** a wall-clock timeout on the activity, tuned for a working agent rather than a
  dispatcher. The *job* re-checks completion after the process exits; that check is authoritative.

The worktree-mismatch guard is the pattern to copy from `agent_implement`: compare `pwd -P` and
`git rev-parse --show-toplevel` against the supplied roots before any write, and fail rather than
write somewhere else.

Conversation resume across fires stays out of scope. Each invoke starts fresh from Orbit state plus
`orbit.session_log.list`.

## 3. `epic_pipeline` job

> **Partly live.** The shipped job runs `resolve_ship_input` → `worktree` → `descendants` → the
> `drain` loop of §1, then stops. The agent step, the completion gate, and delivery are [ORB-10818];
> until they land, an epic run assembles the branch but does not ship it.

```text
worktree = worktree_setup(epic root, run_id: epic-<id>, branch_prefix: epic)
loop
  for each non-terminal descendant, in order:
      invoke_and_wait task_local_pipeline onto the epic branch   # child -> done
  invoke epic_orchestrator in worktree
  break when the epic has no non-terminal descendants and the agent reported complete
until break or max_iterations
if descendants remain: fail closed
deliver:
  mode = pr    -> pr_prepare -> git_rebase -> git_push -> pr_open -> pr_promote
  mode = local -> git_merge epic branch into the workspace base
epic root -> review (pr) | done (local)
```

- **Completion is epic-scoped** ([Epic completion is epic-scoped](./4_decisions.md#epic-completion-is-epic-scoped-not-workspace-scoped), [ORB-10818]). A deterministic activity returns the
  root's non-terminal descendants. Agent prose cannot mark the job successful while any remain, and
  an unrelated `backlog` chore elsewhere in the workspace neither fails nor prolongs the run.
- **Delivery is inlined**, not delegated. `task_pr_pipeline` begins with its own `worktree_setup`;
  invoking it would build a second worktree instead of shipping the epic's. The delivery activities
  all accept `workspace_path`, so they compose directly against the epic worktree.
- An epic that produced **no diff** must not open an empty PR. `skipped_no_diff_expected` in
  `task_pr_pipeline` is the precedent.
- `max_iterations` and the job timeout are fail-closed ceilings, not "close enough."
- `max_active_runs: 1` per epic job so two overlapping fires do not double-orchestrate.

## 4. Workspace drain (`workspace_auto_pipeline`)

> **Planned** ([ORB-10819]). `classify_workspace_auto_tasks` still returns the four-way
> `ship`/`hold`/`epic`/`empty` decision and still short-circuits to `hold` whenever an epic root is
> `in-progress`; there is no window. The epic reservation this section depends on is already live
> (see below).

`orbit run ship` is a leaf implementer. The logistics verb is a separate job; that split from v1
stands. What changes is that a tick becomes a **window** ([Auto drains for a window instead of taking one action](./4_decisions.md#auto-drains-for-a-window-instead-of-taking-one-action), [ORB-10819]).

```text
resolve_ship_input                      # mode + base_branch for this workspace
open_window(input.for_seconds)          # stamps a deadline; absent/zero = one tick
loop
    admissible = backlog leaves whose context_files do not overlap a live holder
               + eligible backlog epic roots
    ship leaves      -> invoke_and_wait task_auto_pipeline
    start epic roots -> dispatch epic_pipeline detached
    sleep when nothing was admissible
    break when the window expired
```

- The deadline gates **starting** new work. In-flight children finish because `invoke_and_wait`
  blocks on them — that is what makes "the window does not affect tasks already in progress" true
  by construction rather than by special-casing.
- Each iteration **re-lists** the backlog, so a task created after the run started still ships.
- Epic dispatch is **detached**. Waiting on a multi-hour epic would consume the rest of the window
  and starve conflict-free leaves behind it, which is the v1 failure mode with a longer fuse. The
  loop re-observes the epic's run state each iteration instead.
- An expired window with nothing left is a plain success. Fail-closed lives at the epic gate (§3),
  not here.

**Conflict admission replaces `hold`.** An `epic`-tagged root holds one reservation covering the
union of its descendants' `context_files`. That union is **live** since [ORB-10816]:
`lock_context_files_for_task` walks `parent_id` to collect every descendant's declared files when
the task carries `tag: epic`, and both `active_task_lock_holders` and `task_overlap_conflicts` read
through it. So loose leaves are already admitted or excluded by machinery that exists —
`task_overlap_conflicts` at discovery, `reserve_locks` atomically at the gate. What [ORB-10819]
still owes is deleting the short-circuit above it, collapsing the four-way
`ship`/`hold`/`epic`/`empty` decision to an admissible set.

A task is in the **epic family** when it carries `tag: epic` or any ancestor does. Walk `parent_id`
the same way `list_backlog_tasks` already walks it for lock grouping.

| Surface | Epic root | Child of that root | Loose leaf |
|---|---|---|---|
| `orbit run ship` (auto / empty ids) | skip (`epic_root`) | skip (`epic_child`) | ship |
| `orbit run ship <id>` / `workflow_ship` | refuse before worktree setup | ship | ship |
| `scan_unresolved_work` | include | include | include |
| `epic_pipeline` | owns it | lands it into the epic branch | not its concern |
| `orbit run auto` | start when admissible | never auto-ship | ship when conflict-free |

Worked example: 2 loose tasks, 1 epic root, 3 epic children, all `backlog`. One
`orbit run auto --for 1h` ships the 2 loose tasks and starts the epic. The epic's 3 children land
into the epic branch one after another, the agent finishes the work, and the run delivers one PR.
A 4th loose task created 10 minutes in ships in the same window if it does not overlap the epic's
reserved files.

Do not fold this heuristic into `list_backlog` beyond the exclusion reasons.

## 5. Unresolved-work scan

`scan_unresolved_work` is a deterministic, read-only activity over the source workspace. It
includes every task in `proposed` / `backlog` / `blocked`, every `failed` / `timeout` job-run except
runs of `epic_pipeline` itself ([Drain scan excludes `epic_pipeline` runs](./4_decisions.md#drain-scan-excludes-epicpipeline-runs)), and every unresolved `check_later`
session-log entry. It excludes `in-progress` tasks (a run is already live), `review` / terminal
tasks, `cancelled` runs, and runs still `pending` / `running` / `retrying`.

Output is `task_ids`, `run_ids`, `check_later_ids`, counts, and `empty`. Nothing is mutated. An
empty scan is success, not an error.

It keeps its workspace-wide shape and is **no longer** `epic_pipeline`'s completion gate (§3). It
answers "does this workspace still have unresolved work", which is a drain question.

## 6. Session log

The agent has no retained CLI conversation. It needs a notebook that survives a fresh invoke:
status of the last fire, notes to self, and "check this later."

That is `orbit.session_log`, a **workspace-scoped, append-only** store. It is not a task, not a
comment thread, and not a knowledgebase markdown file (the drain runs in the *target* workspace;
the cron repo is the wrong disk).

Each entry:

| Field | Rule |
|---|---|
| `id` | allocated, stable |
| `at` | timestamp |
| `kind` | `status` \| `note` \| `check_later` |
| `body` | markdown |
| `related_task_ids` / `related_run_ids` | optional |
| `resolved_at` | set only on `check_later` via `resolve` |

Tools: `orbit.session_log.append`, `orbit.session_log.list` (filters `kind`, `unresolved_only`,
`since`), `orbit.session_log.resolve`.

`status` and `note` never wake the scan. Unresolved `check_later` **does** — that is how "remind me"
works without session resume. Resolving a check-later is how the agent tells the next scan the
reminder is done.

Rejected shapes: stuffing JSON into task comments (rejected with ORB-10778); a standing `backlog`
"log task" (it would wake the drain forever); a file in the knowledgebase cron repo (wrong
workspace).

Do not rewrite history. Append or resolve; never edit a body in place.

## 7. External clock (not an Orbit routine)

Orbit does not seed `resident-epic-orbit` or any other routine for this feature. A knowledgebase
checkout (or the front-door orchestrator) fires:

```text
orbit run auto --for 1h
# or, to drive one epic directly:
orbit run job epic_pipeline
```

via MCP or CLI, on a cron it owns. That process may also create `epic`-tagged roots, author
children, and answer humans. None of that is a seeded routine.

`workspace_ship_pipeline` remains the stable wrapper name for existing sweeps and delegates to the
sequencer. It waits on its child, so whatever window it passes is a window its routine's
`overlap: forbid` holds for. Choose that window deliberately: `ship-sweep-orbit` fires every 30
minutes.

## 8. Authority and completion

Daniel's merge authority is unchanged. An epic delivers a PR; it does not merge one.

`review` is not a wake reason: a task waiting on human merge must not keep a drain alive by itself.
If the only leftovers are `review` plus healthy runs and no unresolved `check_later` notes, the
scan is empty.

A failed child pipeline that flipped its task to `blocked` *is* a wake reason. Inside an epic, the
agent is expected to diagnose it directly — it has the worktree and the tools. If it cannot
progress, it leaves a precise block reason and exits so the completion gate still sees the item and
the job fails at the ceiling rather than lying.

## 9. Concerns & Honest Limitations

- **One long run owns the slot.** `max_active_runs: 1` plus `--for 2h` means two hours of
  occupancy, inherited by `workspace_ship_pipeline` and its routine.
- **Children are `done` before anything lands on the base.** A child is `done` when its commits are
  on the *epic* branch, which also makes its worktree gc-eligible. If the epic is later abandoned,
  that work is `done` in Orbit and unlanded in git. Abandoning an epic is a human act; it needs a
  deliberate recast of its children.
- **Sequential children are slower.** An epic with ten independent children pays ten serial child
  runs. That is the price of one branch and one fast-forward lane.
- **The epic reservation is coarse.** The union of descendants' `context_files` is the exclusion
  set, and `context_files` is a declared boundary, not a proof. An epic that edits something it
  never declared can still collide with a leaf that ran concurrently.
- **Conflict admission is all-or-nothing.** Any overlap excludes a leaf. A tolerance threshold —
  "N conflicting files are acceptable" — is a later feature, as is `orbit run ship --force` for
  overriding the gate on an explicitly named task.
- **A code-editing agent is a bigger blast radius** than a dispatcher. The worktree-mismatch guard
  and the sandbox profile are the only things keeping it inside its worktree.
- **Workspace-wide drain.** One `blocked` chore anywhere still wakes the scan for the whole
  workspace. Isolation is a workspace-layout problem, not a tag filter.
- **Two dispatchers on one workspace.** `orbit run ship` and `orbit run auto` still share untagged
  backlog if someone fires both. Prefer `orbit run auto` as the single auto entry.
- **No conversation continuity.** Every fire re-reads Orbit and the session log.
- **Ceilings can fail a job with work left.** That is correct. The next fire tries again. Do not
  weaken the post-loop completion check.

## 10. What v1 did and why it changed

Recorded so a reader of the shipped code can tell the two apart while [ORB-10815] lands child by
child. The banner table at the top says which rows below are already true of the shipped code.

| v1 | v2 | Why |
|---|---|---|
| `epic_pipeline` looped `scan_unresolved_work` -> `epic_orchestrator` over the whole workspace | epic-scoped drain over one root's descendants, then delivery | the scan failed an epic for unrelated backlog and passed one whose own children were unfinished |
| `epic_orchestrator` could not edit the repository | works inside the epic worktree with subagents | one epic fragmented into N worktrees, N PRs, N review items |
| Children shipped independently to the workspace base | children land into the epic branch sequentially, `done` on merge | nothing ever reviewed the epic's combined result |
| `epic_pipeline` never delivered | inlined PR or local merge of the epic branch | delivery only ever existed per-child |
| Auto tick took exactly one action | drain window with repeated re-listing | work created after classify waited for the next external fire |
| `decision: hold` froze auto-ship during an epic | epic reservation over the descendant context union | conflict-free chores had no reason to wait |
| `run_epic` used `invoke_and_wait` | detached dispatch, re-observed each iteration | a multi-hour epic starved every leaf behind it |

## Task References

- **[ORB-10332]** — Removed HTTP epic pipeline.
- **[ORB-10775]** — v1 implementation epic.
- **[ORB-10776]** — v1 contract and its decisions.
- **[ORB-10779]** — v1 scan, orchestrator activity, `epic_pipeline`.
- **[ORB-10784]** — `orbit.session_log`.
- **[ORB-10788]** — v1 sequencer, leaf-ship exclusion, `orbit run auto`.
- **[ORB-10815]** — This revision.
- **[ORB-10816]** — §1 epic worktree, sequential child drain, epic reservation.
- **[ORB-10817]** — §2 epic agent.
- **[ORB-10818]** — §3 completion gate and delivery.
- **[ORB-10819]** — §4 drain window and detached dispatch.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
