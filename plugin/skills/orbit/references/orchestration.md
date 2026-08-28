# Orchestrating work

Driving a backlog through Orbit rather than executing one task by hand. Read
[workflows.md](workflows.md) first for what jobs, activities, and runs are; this
covers the choices an orchestrator makes on top of them.

## Entry points

```bash
orbit run ship                      # ship ready backlog tasks through the gated pipeline
orbit run ship <task-id> ...        # ship exactly these
orbit run ship --mode local         # commit to the current branch; no PR
orbit run auto --for 2h             # drain the backlog for a window
orbit run ship-sweep --dry-run      # what every registered workspace would ship
orbit run triage                    # diagnose tasks blocked by failed runs
```

`ship` with no IDs discovers ready backlog work itself. `--mode` defaults to the
workspace's registered ship mode (`pr` unless set otherwise), and `--base`
defaults to `workflow.base_branch`.

`run auto` drains loose leaf tasks plus one epic for a bounded window. The window
bounds only the *start* of new work — a task already shipping when it expires
still finishes.

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

This is cheap relative to what it prevents: the pilot is read-only, runs five
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

When two operators or hosts could dispatch into the same workspace, one holds an
exclusive claim and the other must present its token:

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
