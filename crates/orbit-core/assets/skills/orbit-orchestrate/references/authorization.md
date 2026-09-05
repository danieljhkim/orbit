# Completion authorization

This assumes [workflows.md](../orbit/references/workflows.md) and
[orchestration.md](../orbit/references/orchestration.md)'s "Delivery and
completion" section — read those for the full `completion` input and
`review -> done` mechanics. This page covers what an orchestrator does with
that authority once granted, not the mechanism itself.

## Base and ship defaults, not machine specifics

Dispatch reads `workflow.base_branch` and the workspace's registered ship mode
(`pr` unless the workspace sets otherwise) rather than a branch or path named
in conversation. Teach and rely on those configured defaults; do not hardcode
a specific branch name, worktree path, or crew name into a runbook — a
different workspace configures different ones; use `--base`/`--mode` only to
deliberately override.

## What authorization actually grants

Every authorization is scoped to what the user actually said, never inferred
from a prior similar request:

- Creating a task, or promoting it to `backlog`, authorizes neither dispatch
  nor completion.
- `orbit run ship <id> --complete` / `orbit run auto --complete` authorize
  exactly the submitted run to carry the tasks it ships from `review` to
  `done` — never a standing default, never something a workspace config file
  or an unattended routine such as `ship-sweep` can turn on.
- `run auto --complete` covers the whole window, including work filed after
  the drain starts — the operator asked for a window, so treat everything the
  drain admits inside it as covered, but not one moment beyond the window's
  own bound (see below) and never a different drain the user did not invoke.
- Never carry a specific session's bounded window (a duration, a task list, a
  crew restriction) forward into a later request as if it were durable
  policy. Ask again; a duration or scope is a fact about one request, not a
  standing grant.

## Keep independent work draining

Once the user has authorized continuous completion for a window, do not add a
blocking pre-merge review phase in front of it — that silently converts an
authorized auto-drain into a gated one the user did not ask for. Post-merge
review, QA, and CI feed findings back into new repair tasks that go through
the ordinary preparation loop; they are not a checkpoint the drain waits on.
→ [recovery.md](recovery.md)

`--allow-crew` is the lever for a provider that is unavailable, rate-limited,
or out of budget mid-window: it excludes named crews from *new* dispatch in
that run only, and a task whose crew is excluded is skipped, not remapped —
reassigning it to a permitted crew is a separate operator decision. See
[orchestration.md](../orbit/references/orchestration.md) for the exact
semantics before relying on it. → [walkthroughs.md](walkthroughs.md)

## Stopping a bounded run

A run's window bounds only the start of new work; a task already shipping
when it expires keeps running to completion — do not assume expiry stopped
it, and do not cancel it merely because the window ended. To stop
in-flight work deliberately:

```bash
orbit run show <run_id> --json     # confirm what is actually still running
orbit run cancel <run_id>          # the owning host's authoritative cancel
```

If cancellation itself needs process-level verification, follow the
diagnose-before-kill order in [run-debugging.md](../orbit/references/run-debugging.md)
rather than sending a signal based on a guess. Never cancel a parent
gate/auto/epic run without first confirming it owns the task(s) you actually
mean to stop.

## Reporting handoff

Report from durable state, not from what you remember doing:

- Merged/completed work: tasks at `done`, with their run ID and verified
  merge evidence.
- Remaining work: tasks still `backlog`/`in-progress`/`review`, with what
  each is waiting on.
- Failures: the task and run ID, and the failure class from
  [run-debugging.md](../orbit/references/run-debugging.md), not a paraphrase.

Never invent a token-cost or spend figure — Orbit does not hand you one, and a
plausible-looking number is worse than omitting it.

A resumable handoff names, at minimum: the workspace selector, any run IDs
still in flight, the authorization that was actually granted (what window,
what scope, `--complete` or not), and the next preparation-loop step for each
task that isn't done. A session that ends without recording this in task/run
state (a task comment, an execution summary) leaves the next orchestrator
guessing exactly like reading only agent prose would.
