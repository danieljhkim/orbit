# Walkthroughs

Seven short situations an orchestrator runs into mid-loop. Each names what
actually happened, the tools that surface it, and the response — not a script
to follow blindly.

## 1. Missing context

A task reaches promotion time with an empty `context_files` and the
description alone doesn't tell you what it would touch.

- Do not guess selectors to fill the field — a wrong entry actively misleads
  conflict detection. → [task-authoring.md](../orbit/references/task-authoring.md)
- Run (or re-run) `task_pilot_pipeline` against exactly that task ID; it
  inspects the pinned revision read-only and applies only what it validated.
- If the pilot itself reports it cannot resolve a target, or the task's
  problem statement is too vague to inspect, stop and comment on the task
  asking for the missing detail rather than promoting an unprepared task.

## 2. Duplicate or already-landed pilot warnings

Pilot apply returns with a `duplicate` or `already-landed` warning attached to
a proposed task.

- Read the warning's cited task/commit before acting on it —
  `orbit tool run orbit.task.show --full`.
- If the warning is correct, do not promote. Close or re-author the task
  (link it to the task it duplicates, or drop it if the work is truly done);
  promoting anyway just races the same file a second time.
- If the warning is a false positive — a genuine near-miss, not the same
  change — say so in a task comment before promoting, so the next reader
  doesn't re-litigate the same question.

## 3. Dependency or lock blockage

`orbit run readiness` reports a task waiting on an unmet dependency ID, or a
context-lock holder blocking it.

- An unmet `dependencies` entry: check that prerequisite task's own status;
  it has to reach a satisfying status before this one is eligible, and
  reordering the backlog doesn't skip that.
- A stale lock with no owning run: match the
  [common-failures.md](../orbit/references/common-failures.md) "Stale Task
  Lock Reservation" signature exactly before touching anything —
  `orbit task locks list`, confirm the blocking task is not actually active,
  then `orbit task locks release <reservation_id>`. Never edit the
  reservation store directly.
- A live lock held by a task that is genuinely still running is not a bug;
  wait for it or reprioritize other work instead.

## 4. Unavailable operator capability

A dispatch or resume call needs operator authority the current session
doesn't have, or the connected MCP server simply doesn't advertise the
operation.

- Confirm with the server's own `tools/list` or `orbit tool list` before
  assuming — a similar-sounding name is not evidence the operation exists.
  → [tool-surface.md](../orbit/references/tool-surface.md)
- Report the missing capability and what it blocks. Do not retry through a
  different host's CLI, a raw store read, or any path that bypasses the
  authority check — that is the shadow-store failure mode, not a workaround.
- A managed leaf run cannot dispatch or resume another run under any
  circumstance; that's a role boundary, not a missing feature to route
  around.

## 5. A CI finding after merge

A `ci_failure_sweep_pipeline` finding names a task that already reached
`done`.

- Do not hand-patch the merged branch to make the finding go away. Let the
  finding file as `proposed` (or file it yourself if the sweep hasn't run
  yet), and route it through the ordinary loop: search for a duplicate,
  confirm the pilot's applied context and warnings, promote, then dispatch as
  its own task. → [recovery.md](recovery.md)
- If the same finding recurs against a series of merges, that pattern itself
  is worth a task (fix the root cause, not each instance) — not an
  ever-growing set of one-off repairs.

## 6. A provider failure mid-window

A crew's provider starts failing, rate-limiting, or running out of budget
while an authorized `run auto --for <duration>` window is still open.

- Start a fresh `run auto --for <duration> --allow-crew <remaining crews>`
  window rather than editing the running one — the exclusion only governs
  what a drain starts, and it is scoped to that run.
- Tasks on the excluded crew are skipped, not remapped, and stay in
  `backlog`. Reassigning one of them to a different crew is an explicit
  operator decision (`orbit.task.update`) — do it only for tasks the user
  wants to move, not the whole backlog by default.
- `orbit run readiness --allow-crew <remaining crews>` shows exactly which
  tasks would be skipped as `crew_not_allowed` before you commit to the new
  window. → [orchestration.md](../orbit/references/orchestration.md)

## 7. Window expiry with in-flight work

An authorized window (`run auto --for <duration>`) ends while tasks are still
shipping.

- The window bounded only when new work could *start*; anything already
  shipping keeps running to completion under its original authorization —
  confirm with `orbit run show <run_id>` rather than assuming it stopped.
- Do not cancel in-flight runs just because the window closed; that discards
  work the user already authorized.
- Continuing to drain *new* work past the window needs a fresh, explicit
  authorization for a new window — the expired one does not silently renew,
  and the new request should not assume the same scope (duration, crews,
  `--complete`) the previous one had. → [authorization.md](authorization.md)
- Report the window's outcome from durable state: what finished, what's still
  running, and what's untouched in `backlog` because the window ended before
  it started.
