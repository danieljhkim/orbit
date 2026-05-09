---
name: orbit-debug-job-failure
description: Use when investigating failed, stuck, cancelled, or suspicious Orbit job runs. Guides agents through run state, step summaries, v2 audit events, logs, blobs, process state, task linkage, recovery/follow-up, and safe reporting.
---

# Orbit Debug Job Failure

## Purpose

Debug an Orbit job run without guessing. A failed run has multiple layers of evidence: the job-run bundle under `.orbit/state/job-runs/`, v2 audit events under `.orbit/state/audit/v2_loop/`, transcript blobs under `.orbit/state/audit/blobs/`, task records, Git state, and sometimes live processes. This skill gives agents a repeatable order of operations so they identify the first real failure, separate root cause from downstream fallout, and report a concrete next step.

## When To Use

Use this skill when the human provides a `jrun-*` id, says a job failed, asks why a run is stuck, asks which task a run is handling, asks whether to kill a run, or asks for a failure diagnosis involving Orbit activities/jobs.

Do not use it for ordinary task implementation unless the implementation request is specifically about a failed Orbit run.

## Safety Rules

- Do not edit files under `.orbit/state/job-runs/` or `.orbit/state/audit/` to "fix" a run. Treat them as evidence.
- Do not kill a process until you have matched the run id to its `pid`, task id(s), process group id, and command. Prefer terminating the process group for that run id only.
- Do not kill parent auto/gate runs unless you verify they are for the same task(s) and leaving them alive would keep the unwanted workflow active.
- Do not rely on a top-level `state: failed` alone. Find the first failed step/activity and the related error message.
- Do not parse agent prose as the durable handoff when task state, run state, or `orbit.state.*` records exist.
- If Orbit tooling, skill guidance, or diagnostics are misleading or broken, file a self-reported friction task with the `orbit-track-issues` skill.

## Quick Triage

Given a run id `<run_id>`:

1. Locate the run bundle:

   ```bash
   find .orbit/state/job-runs -maxdepth 3 -type d -name '<run_id>' -print
   ```

2. Read the run manifest and state:

   ```bash
   sed -n '1,140p' .orbit/state/job-runs/<job_id>/<run_id>/jrun.yaml
   sed -n '1,220p' .orbit/state/job-runs/<job_id>/<run_id>/state.json
   find .orbit/state/job-runs/<job_id>/<run_id>/steps -maxdepth 1 -type f -print -exec sed -n '1,220p' {} \;
   ```

3. Record these fields before drawing conclusions:

   - `job_id`
   - `state`
   - `pid` and `pid_start_time`
   - `input.task_ids`, `input.base_branch`, `input.base_sync`, and mode flags
   - `started_at`, `finished_at`, and `duration_ms`
   - failing `step_id`, `activity_name`, `error_message`, and any recovery attempt

4. If there are multiple candidate run ids, compare their `input.task_ids` first. This is the fastest way to identify which run owns a task.

## Use Orbit Inspection Commands First

Prefer the public inspection surface before raw file spelunking:

```bash
orbit run show <run_id> --json
orbit run events <run_id> --json
orbit run trace <run_id>
orbit run logs <run_id> --json
```

Use step-scoped variants when the failing step is known:

```bash
orbit run show <run_id> -s <step_id> --json
orbit run logs <run_id> -s <step_id> --json
orbit run events <run_id> -s <step_id> --json
```

If these commands fail or omit needed detail, fall back to the files under `.orbit/state/` and mention that fallback in your report.

## Read The V2 Audit Trail

The audit file is usually:

```bash
.orbit/state/audit/v2_loop/<run_id>.jsonl
```

Use it to reconstruct the exact sequence:

```bash
tail -80 .orbit/state/audit/v2_loop/<run_id>.jsonl
rg -n 'failed|error|recovery|cli.invocation|step.started|step.finished|activity.started|activity.finished|run.finished' .orbit/state/audit/v2_loop/<run_id>.jsonl
```

Interpretation guide:

- `run.started` / `run.finished` define the overall lifecycle.
- `step.started` / `step.finished` identify job step boundaries.
- `activity.started` / `activity.finished` identify activity execution and deterministic vs agent-loop type.
- `cli.invocation.started` / `cli.invocation.finished` identify provider command, model, cwd, timeout, exit code, and stdout/stderr blob refs.
- `step.recovery_attempted` tells whether recovery ran and whether it succeeded.
- A failed recovery can be a secondary problem; diagnose the original failed step first.

## Read Logs And Blobs

`orbit run logs` is the preferred way to read captured stdout/stderr. If you need raw blobs, map blob refs through `.orbit/state/audit/blobs/<first-two-hex>/<full-hash>`.

Example:

```bash
blob=<blob_ref>
sed -n '1,220p' ".orbit/state/audit/blobs/${blob:0:2}/$blob"
```

For large agent stdout blobs, use targeted search first:

```bash
rg -n 'error|failed|panic|conflict|Validation|Outcome|execution_summary|git push|pr_open|rebase|ModelNotFound' .orbit/state/audit/blobs/<hh>/<blob>
```

Do not paste huge transcripts back to the human. Summarize the decisive lines and identify the blob or command source.

## Distinguish Failure Classes

Classify the failure before suggesting a fix:

- **Implementation failure:** the agent loop exited nonzero or reported a failed envelope during `implement_one`.
- **Validation failure:** implementation completed but `make build`, `make fmt`, `cargo test`, or a task-specific command failed.
- **Git/branch failure:** `git_push`, `pr_open`, `git_merge`, freshness checks, rebase, or conflicts failed after implementation.
- **Provider/tooling failure:** provider command failed before useful work, model unavailable, timeout, sandbox denial, or tool surface mismatch.
- **Recovery failure:** the original step failed and `step_failure_recovery` also failed. Report both, but keep the original step as primary unless recovery caused additional damage.
- **Parent orchestration failure:** a child run failed and a gate/auto/epic parent is still running or waiting. Identify both run ids.

## Check Task State

When a run has task ids, inspect the relevant tasks through Orbit tools:

```bash
orbit tool run orbit.task.show --full --input '{"id":"<task_id>","model":"<model_name>"}'
```

Check:

- status and history
- plan and execution_summary
- comments and review_threads
- workspace_path
- external_refs / PR metadata
- dependencies and resolved_dependencies

If implementation succeeded but later workflow failed, the task may already contain a useful execution summary. Preserve that context in your diagnosis.

## Check Parent And Child Runs

Parent gate/auto/epic runs can fail because a child run failed, and child runs can keep working after a parent reports a gate failure. Identify both sides before reporting:

```bash
rg -n '<run_id>|<task_id>' .orbit/state/job-runs .orbit/state/audit/v2_loop
orbit run history --json
```

Look for:

- `input.task_ids` overlap between candidate runs
- parent events that invoke or wait on another `jrun-*`
- child run ids named in gate, auto, epic, or `invoke_and_wait` step output
- parent runs still in `pending` or `running` after a child has failed

Report the run that owns the first real failure as primary, then name any parent/child fallout separately.

## Check Git State For Workflow Failures

For branch/push/PR failures, inspect the worktree recorded by the run:

```bash
git -C <workspace_path> status --short --branch
git -C <workspace_path> rev-parse --abbrev-ref HEAD
git -C <workspace_path> rev-list --left-right --count <base_ref>...HEAD
git -C <workspace_path> log --oneline --decorate --graph --max-count=12 --all
```

Use `git merge-tree` or a dry-run rebase only to understand conflicts. Do not resolve conflicts unless the human asked you to fix the run, not merely investigate it.

For remote questions, compare local and remote refs explicitly:

```bash
git -C <workspace_path> ls-remote origin refs/heads/<branch> refs/heads/<base_branch>
```

## Check Live Processes

If `jrun.yaml` says `state: running`, verify whether the recorded process still exists and whether its start time matches the run:

```bash
ps -o pid,ppid,pgid,stat,etime,command -p <pid>
ps -axo pid,ppid,pgid,stat,etime,command | rg '<run_id>|<workspace_path>|<task_id>'
```

If the human asks you to kill a run:

1. Match run id -> task id(s) -> `pid` -> `pgid` -> command.
2. Prefer process-group termination:

   ```bash
   kill -TERM -<pgid>
   sleep 2
   ps -axo pid,ppid,pgid,stat,etime,command | awk '$3==<pgid> {print}'
   ```

3. If children remain and the human clearly asked to kill the run, escalate to `kill -KILL -<pgid>`.
4. If the killed child belongs to a parent gate/auto run for the same task, inspect the parent. Kill the parent only after verifying it owns the same task(s) and is keeping the unwanted workflow alive.
5. Report whether the run record updated to `failed`, `cancelled`, or still says `running` despite no live process.

## Report Format

Lead with the answer, then evidence:

```markdown
<run_id> failed in <step_id>/<activity_name>.

Primary cause: <one sentence>.

Evidence:
- Task(s): <ids>
- Run state: <state>, started <timestamp>, finished <timestamp or still running>
- First failed event: <event type / step / activity>
- Key error: <short error text>
- Relevant stdout/stderr/audit source: <command, blob ref, or file path>

Current state:
- Process: <not running | running pid/pgid | killed>
- Task: <status and important metadata>
- Branch/PR: <if relevant>

Next step: <specific recommended action>
```

Keep the report short unless the human asks for a full forensic trace.

## Validation Checklist

Before finalizing a diagnosis, verify:

- You matched the right run id and task id(s).
- You identified the first failed step, not just the last logged error.
- You checked recovery events when present.
- You checked stdout/stderr blobs for the failing invocation when available.
- You checked live process state for runs marked `running`.
- You separated root cause from downstream fallout.
- You filed a friction task if Orbit diagnostics or recovery behavior were misleading.
