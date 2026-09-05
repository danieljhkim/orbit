# The preparation loop

Discover tools and workspace authority before acting — call
`orbit_workspace_list`/`orbit workspace list` first and use its returned
selector, exactly as [tool-surface.md](../orbit/references/tool-surface.md)
describes. Everything below assumes that selector is in hand.

## 1. Inspect workspace goals and live evidence

Read the actual current state before deciding what needs doing. Task and run
state are the evidence; a prior conversation's summary is not:

```bash
orbit tool run orbit.task.list --input '{"workspace":"<selector>","status":"backlog,in-progress,review,blocked","model":"<agent-family>"}'
orbit run readiness --json                 # who is eligible to drain right now, and why not
orbit run history --json                   # recent runs, terminal and in-flight
```

`orbit run readiness` never mutates anything — read it as often as needed while
deciding what to prepare next.

## 2. Search open and closed history

Before authoring anything, check whether the work already exists or was
already done. → [search.md](../orbit/references/search.md)

```bash
orbit search "<problem terms from the goal>" --hybrid --kind task
orbit search "<problem terms>" --kind all --status task:done,doc:active
```

A brand-new task has no vectors yet, so query the *text*, not an ID. Search
closed history too — proposing a repair for something already merged is a
wasted preparation cycle, and a genuinely already-landed case still needs to
be recognized, not silently re-filed. → [walkthroughs.md](walkthroughs.md)

## 3. Author bounded tasks

Write a task the pilot and an executor can act on without guessing: a crisp
problem statement, acceptance criteria naming observable success, and
`complexity`. Full authoring standard: [task-authoring.md](../orbit/references/task-authoring.md).

```bash
orbit tool run orbit.task.add --input '{
  "title": "<title>", "description": "<multi-line markdown>",
  "acceptance_criteria": ["<observable outcome>"],
  "workspace": "<selector>", "priority": "<low|medium|high|critical>",
  "complexity": "<low|medium|hard>", "type": "<feature|bug|refactor|chore>",
  "model": "<agent-family>"
}'
```

This is the **proposed creation** example: the task now exists in `proposed`.
Creating it authorizes nothing — not context preparation, not dispatch, not
completion.

Do not guess `context_files` here to avoid an empty field; an empty list is
valid and a wrong entry actively misleads conflict detection. Leave selectors
to the next step.

## 4. Choose a crew within configured limits

Read what this workspace actually configures and what the user has authorized
for cost and provider before assigning `crew` — `orbit executor list`, the
workspace's `[workflow].default_crew`, and any provider mix the user stated for
this session. Do not invent a crew name, and do not carry a crew choice from
one session into another as if it were durable policy; it is a per-task or
per-run field, and asking again costs nothing.

## 5. Run task_pilot_pipeline before promotion

Do not fill `context_files` by inline guessing. The task-pilot job inspects a
pinned revision read-only and applies only the selectors it validated:

```bash
orbit run job task_pilot_pipeline                                     # zero-input discovery
orbit run job task_pilot_pipeline --input 'task_ids=["<task-id>"]'    # audit exactly these
```

This is the **pilot apply** step. Its durable output — read with
`orbit run show <run_id> --json` — reports each partition as `applied`,
`skipped_stale`, or `failed`, plus proposals and duplicate,
already-landed, dependency, and conflicting-decision warnings. Full mechanics:
[orchestration.md](../orbit/references/orchestration.md).

## 6. Inspect applied context and warnings before promotion

Read the pilot's applied selectors and warnings on the task itself before
moving it forward:

```bash
orbit tool run orbit.task.show --full --input '{"id":"<task-id>","model":"<agent-family>"}'
```

Only once the applied `context_files` and warnings look right does promotion
happen — an explicit lifecycle transition, not something the pilot performs
for you:

```bash
orbit tool run orbit.task.update --input '{"id":"<task-id>","status":"backlog","model":"<agent-family>"}'
```

This is the **backlog promotion** example. A duplicate, already-landed, or
unresolved-dependency warning is a reason to stop here, not to promote and
hope. → [walkthroughs.md](walkthroughs.md)

## 7. Dispatch under explicit authorization

Promotion to `backlog` still does not authorize a run. Dispatch only when the
user has authorized it, using the entry points in
[orchestration.md](../orbit/references/orchestration.md):

```bash
orbit run ship <task-id> ...          # this is the submission example
orbit run auto --for <duration>       # or drain the backlog for a bounded window
```

Submission is asynchronous: the command returns once the run is durable, not
once anything landed.

## 8. Supervise, then feed back

Poll durable state, not agent prose:

```bash
orbit run show <run_id> --json
orbit tool run orbit.task.show --input '{"id":"<task-id>","model":"<agent-family>"}'
```

A task that reaches `review` with a verified merged PR (**merge**) is distinct
from one an authorized completion run has also carried to `done`
(**done**) — confirm both independently rather than inferring one from the
other. → [authorization.md](authorization.md)

Post-merge review comments, QA findings, and CI failures are new input to this
same loop, starting again at step 2 — not a special path with different rules.
→ [recovery.md](recovery.md)
