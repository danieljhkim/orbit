## Context

Delivery refuses to hand off a task whose durable `execution_summary` is empty: `reject_failed_delivery` rejects an empty or placeholder summary before the commit step touches the index (ORB-10313), and `update_task_with_status_note_and_identity` refuses the `in-progress -> review` transition on the same grounds. The summary is real evidence — the PR body renders it, and reviewers read it.

Nothing in the pipeline ever wrote that field. The deterministic `update_task` action hardcodes `execution_summary: None`, and the only writer was instruction 14 of the `agent_implement` activity, prose asking the implementing agent to persist one. Agents skip it often, and every run that skipped it wedged at commit with a change sitting uncommitted in the worktree.

## Decision

The commit step derives the summary from the change it is about to deliver, and only when durable state carries none.

1. `commit_batch_changes` calls `ensure_durable_execution_summary` after read-only checkout resolution and validation, and before the delivery gate. It no-ops when `meaningful_execution_summary` already finds one, so an agent-authored summary always wins.
2. The derived text is read out of `git status --porcelain=v1 --untracked-files=all -z` in the delivery worktree — the same file set `git add --all` will stage — and names each path with its change kind, capped at 25 entries plus a remainder count. It claims no outcome, only what the diff shows.
3. It is persisted to the task record through `apply_task_automation_update` with a `execution_summary_derived` event, so it is durable before any Git mutation and re-checkable afterwards with `git show --stat` on the delivery commit.
4. When there is no change to describe, nothing is derived and nothing is persisted; the gate rejects as before.

The gate's contract is untouched. What changed is that its rejection is no longer reachable in the ordinary case.

### Rejected alternatives

- *Lift the summary out of the agent's returned envelope.* It would satisfy the gate with one line of code, and it is a doctrine violation (L-0115): agent-loop output is advisory, the runner states provider output is not the system of record, and the activity's own instruction says the returned object is not persisted. A pipeline decision must not read it.
- *Relax the guard on the local path.* Deletes the evidence rather than producing it. Downstream consumers, the PR body included, read this field.
- *Derive in the `update_task` action at `mark_review`.* Too late: commit gates first, so the run still wedges before delivery, and that action would then be in the business of authoring task content.
- *Derive after staging, from `git diff --cached --numstat`.* Gives line counts, but only by mutating the index before the delivery gate — the exact ordering ORB-10313 established.

## Consequences

- A task that has been through implementation and commit carries a non-empty summary in durable state whether or not the agent wrote one, so delivery, the `in-progress -> review` transition, and the PR body all have their evidence.
- The `agent_implement` instruction to persist a real summary still stands and still produces the better artifact; the derived one is a floor, not a replacement.
- Checkout resolution and branch/merge validation now run before the delivery gate, since the derived summary reads the worktree the gate protects. Nothing ahead of the gate mutates Git state.
- `Cost:` a derived summary describes the shape of a change, not its intent. A PR whose body carries one tells a reviewer which files moved and nothing about why, which is weaker than an agent-authored account and could be mistaken for one if the opening line is not read.
- `Cost:` the parser is coupled to `git status --porcelain=v1 -z` record framing, including the rename/copy source field that follows its record.
- `Cost:` a task tagged `no-diff-expected` with an empty summary still has nothing to derive from and still fails the gate, unchanged from before this decision.