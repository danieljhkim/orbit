# Orchestrating work

Driving a backlog through Orbit rather than executing one task by hand. Read
[workflows.md](workflows.md) first for what jobs, activities, and runs are; this
covers the choices an orchestrator makes on top of them.

## Entry points

Use `orbit_workflow_ship` with explicit `task_ids`, `workspace`, and attribution
when driving an authoritative MCP connection. Observe with
`orbit_workflow_run_show/list`; resume eligible terminal work with
`orbit_workflow_run_resume`, which returns a new linked run. These operations
require operator authority. Managed leaf runs cannot dispatch follow-up runs.
See [tool-surface.md](tool-surface.md).

The CLI offers additional discovery modes below. Use them only where the user
and workspace dispatch policy permit auto-discovery; creating tasks or enabling
a filing routine does not itself authorize execution.

```bash
orbit run ship                      # ship ready backlog tasks through the gated pipeline
orbit run ship <task-id> ...        # ship exactly these
orbit run ship --mode local         # implement in a worktree, merge to the base; no PR
orbit run auto --for 2h             # drain the backlog for a window
orbit run auto --for 2h --concurrency 8   # ... with 8 tasks in flight at a time
orbit run ship <task-id> --complete  # ... and also carry it through to `done`
orbit run ship-sweep --dry-run      # what every registered workspace would ship
orbit run triage                    # diagnose tasks blocked by failed runs
```

`ship` with no IDs discovers ready backlog work itself. `--mode` defaults to the
workspace's registered ship mode (`pr` unless set otherwise), and `--base`
defaults to `workflow.base_branch`.

`run auto` drains loose leaf tasks plus one epic for a bounded window. The window
bounds only the *start* of new work — a task already shipping when it expires
still finishes.

It keeps `--concurrency` tasks in flight (5 by default) and re-lists the whole
backlog every pass, so a slot is refilled as soon as its own task finishes and a
task filed mid-window starts without waiting for the batch around it.

Runs are asynchronous: these commands return once the run is durable, printing a
run ID. They do not claim the eventual outcome.

```bash
orbit run history -j task_auto_pipeline
orbit run show <run_id>
```

## Prepare selectors before dispatching under traffic

`context_files` is what conflict detection and file reservation read. Tasks with
an empty list can be dispatched, but they cannot be kept off each other's files
— so under high ship traffic, fill them first.

Do **not** fill them inline. Use the task-pilot job: it audits tasks read-only in
bounded partitions, and its apply step persists only selectors it validated.

```bash
orbit job show task_pilot_pipeline
orbit run job task_pilot_pipeline                                  # zero-input discovery
orbit run job task_pilot_pipeline --input task_ids=<id>,<id>       # audit exactly these
```

Zero-input mode discovers only `proposed`/`backlog` tasks in the invoking
workspace whose `context_files` is empty, and skips tasks tagged as needing no
diff. Explicit `task_ids` audits exactly the named tasks, including ones that
already have selectors.

The pilot agent inspection is read-only; its deterministic apply step mutates
validated task selectors. It runs five
partitions concurrently, and returns selector proposals plus duplicate,
already-landed, dependency, and conflicting-decision warnings. An enabled
workspace routine may already run the zero-input job every few hours — an extra
run before a large dispatch is still appropriate.

## Keeping parallel runs off each other

- **Reservation is the system's job, not a worker's.** There is no
  worker-callable lock tool and none should be reached for.
- **Conflict detection reads live from in-flight tasks**, not only from
  reservation records — which is why current `context_files` matter more than
  they look.
- **A selector added mid-run binds only reservations requested after it.** It
  cannot retroactively revoke a reservation a concurrent run already holds.
- **Inspect and repair stale reservations** with `orbit task locks list` and
  `orbit task locks release <reservation_id>` — never by editing the store. The
  signature to match first is in [common-failures.md](common-failures.md).

## Epics

An epic is a parent task with descendants. `epic_pipeline` gives the whole family
one stable worktree and branch, landing unfinished descendants against it rather
than opening a worktree per child. Completion is epic-scoped: descendants left
over at the iteration ceiling fail the run closed rather than silently passing.

`orbit run auto` picks up one epic per window alongside loose leaves.

## Triage

```bash
orbit run triage                    # every blocked task attributable to a failed run
orbit run triage <task-id> ...      # narrow the scan
```

Triage separates environmental casualties — a transient lock, a provider timeout,
a sandbox denial — from real failures. The former are re-backlogged; the latter
stay blocked with a diagnosis attached. Running it on a schedule is what keeps a
failed run from quietly parking a task forever. → [automation.md](setup/automation.md)

## Multi-operator workspaces

When two operators act on the same authoritative workspace store, one can hold
an exclusive claim and another must present its token. Claims do not coordinate
independent stores on different machines. These examples present an existing
token; they do not acquire a claim:

```bash
orbit run ship --claim-token <token>
ORBIT_WORKSPACE_CLAIM_TOKEN=<token> orbit run auto --for 1h
```

For splitting work across machines so their task IDs and schedules don't collide
in the first place, see [multi-host.md](setup/multi-host.md).

## Unattended shipping

`ship-sweep` and the `ship-sweep` routine dispatch without a human present. Both
require `workflow.auto_ship = true`. Before enabling either:

- Confirm `workflow.base_branch` points where PRs should actually land.
- Enable worktree GC first — unattended shipping is the fastest way to fill a
  disk with abandoned worktrees. → [maintenance.md](setup/maintenance.md)
- Watch `orbit run ship-sweep --dry-run` across a realistic backlog first.

## Handoff discipline

Task state, run state, and the durable stores are the handoff — never agent
prose. An orchestrator that reads a summary paragraph instead of
`orbit.task.show` or `orbit run show` is guessing.

## Delivery and completion

By default the task pipelines end in `review`. PR mode prepares a source branch
and opens a PR; that is not evidence it merged. Local mode implements in an
isolated worktree and fast-forwards the configured local base branch; the leaf
job's `auto_push` input controls its optional push. Inspect the effective wrapper
and child job inputs rather than assuming local mode means the current checkout
was edited or a remote branch was updated.

Record validation, commit, branch/PR, and run evidence, then follow the user's
approval policy for completion. Task snapshot publication is independent of
source delivery and lifecycle.

`orbit run ship --complete` and `orbit run auto --complete` are the operator's
explicit authorization for one submitted run to finish delivery and take the
tasks it ships from `review` to `done`. Default-off, and never enabled by
workspace configuration, an environment variable, or an unattended routine such
as `ship-sweep`.

- Local mode completes only after the bundle merged *and* pushed; a failed
  publication leaves the task in `review`.
- PR mode completes only after the PR is verified merged. Branch protections and
  required checks are respected and never bypassed; pending checks may use
  GitHub auto-merge, but enabling auto-merge is not success. A closed or blocked
  PR, a refused auto-merge, or an expired wait leaves the task in `review`.
- Validated `no-diff-expected` work completes without a PR.
- `run auto --complete` is blanket authorization for every task the drain admits
  during its whole window, including work filed after it starts. Do not use it
  where the user authorized only the currently visible backlog.
- It authorizes delivery completion and `review -> done` only. It never approves
  `proposed` work into the backlog and is not an independent review verdict; the
  transition is recorded against the authorizing run and operator.

Submission stays asynchronous, so a `--complete` run's eventual outcome is not
known when the command returns — confirm with `orbit run show <run_id>` and
`orbit.task.show` rather than assuming it completed.
