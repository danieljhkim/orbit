# Debugging a failed job run

Debug an Orbit job run without guessing. A failed run has multiple layers of evidence: the job-run bundle under `.orbit/state/job-runs/`, v2 audit events under `.orbit/state/audit/v2_loop/`, transcript blobs under `.orbit/state/audit/blobs/`, task records, Git state, and sometimes live processes. This gives a repeatable order of operations so you identify the first real failure, separate root cause from downstream fallout, and report a concrete next step.

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

3. Record before drawing conclusions: `job_id`; `state`; `pid` and `pid_start_time`; `input.task_ids`, `input.base_branch`, `input.base_sync`, mode flags; `started_at`/`finished_at`/`duration_ms`; failing `step_id`, `activity_name`, `error_message`, and any recovery attempt.

4. If there are multiple candidate run ids, compare `input.task_ids` first — the fastest way to identify which run owns a task.

## Use Orbit Inspection Commands First

Prefer the public inspection surface before raw file spelunking:

```bash
orbit run show <run_id> --json
orbit run events <run_id> --json
orbit run trace <run_id>
orbit run logs <run_id> --json
```

Step-scoped variants when the failing step is known: `orbit run show|logs|events <run_id> -s <step_id> --json`.

If these commands fail or omit needed detail, fall back to files under `.orbit/state/` and mention the fallback in your report.

## Read The V2 Audit Trail

```bash
tail -80 .orbit/state/audit/v2_loop/<run_id>.jsonl
rg -n 'failed|error|recovery|cli.invocation|step.started|step.finished|activity.started|activity.finished|run.finished' .orbit/state/audit/v2_loop/<run_id>.jsonl
```

Interpretation: `run.started`/`run.finished` define the overall lifecycle; `step.started`/`step.finished` are job step boundaries; `activity.started`/`activity.finished` identify activity execution and deterministic vs agent-loop type; `cli.invocation.started`/`.finished` identify provider command, model, cwd, timeout, exit code, stdout/stderr blob refs; `step.recovery_attempted` tells whether recovery ran and succeeded — a failed recovery can be a secondary problem, diagnose the original failed step first.

## Read Logs And Blobs

`orbit run logs` is the preferred way to read captured stdout/stderr. For raw blobs, map blob refs through `.orbit/state/audit/blobs/<first-two-hex>/<full-hash>`:

```bash
blob=<blob_ref>
sed -n '1,220p' ".orbit/state/audit/blobs/${blob:0:2}/$blob"
rg -n 'error|failed|panic|conflict|Validation|Outcome|execution_summary|git push|pr_open|rebase|ModelNotFound' .orbit/state/audit/blobs/<hh>/<blob>
```

Do not paste huge transcripts back to the human — summarize the decisive lines and identify the blob/command source.

## Distinguish Failure Classes

- **Implementation failure:** the agent loop exited nonzero or reported a failed envelope during `implement_one`.
- **Validation failure:** implementation completed but `make build`, `make fmt`, `cargo test`, or a task-specific command failed.
- **Git/branch failure:** `git_push`, `pr_open`, `git_merge`, freshness checks, rebase, or conflicts failed after implementation.
- **Provider/tooling failure:** provider command failed before useful work, model unavailable, timeout, sandbox denial, tool surface mismatch.
- **Recovery failure:** the original step failed and `step_failure_recovery` also failed — report both, keep the original step as primary unless recovery caused additional damage.
- **Parent orchestration failure:** a child run failed and a gate/auto/epic parent is still running or waiting — identify both run ids.

For recurring signatures and known remedies, read [common_failures.md](common_failures.md) after the initial classification — keep this file focused on investigation flow; add new patterns there.

## Check Task State

```bash
orbit tool run orbit.task.show --full --input '{"id":"<task_id>","model":"<agent-family>"}'
```

Check status/history, plan/execution_summary, comments/review_threads, workspace_path, external_refs/PR metadata, dependencies/resolved_dependencies. If implementation succeeded but a later workflow step failed, the task may already have a useful execution summary — preserve that context.

## Check Parent And Child Runs

Parent gate/auto/epic runs can fail because a child failed, and children can keep working after a parent reports a gate failure:

```bash
rg -n '<run_id>|<task_id>' .orbit/state/job-runs .orbit/state/audit/v2_loop
orbit run history --json
```

Look for `input.task_ids` overlap between candidate runs, parent events that invoke/wait on another `jrun-*`, child run ids named in gate/auto/epic/`invoke_and_wait` step output, and parent runs still `pending`/`running` after a child failed. Report the run owning the first real failure as primary, then name parent/child fallout separately.

## Check Git State For Workflow Failures

```bash
git -C <workspace_path> status --short --branch
git -C <workspace_path> rev-parse --abbrev-ref HEAD
git -C <workspace_path> rev-list --left-right --count <base_ref>...HEAD
git -C <workspace_path> log --oneline --decorate --graph --max-count=12 --all
git -C <workspace_path> ls-remote origin refs/heads/<branch> refs/heads/<base_branch>
```

Use `git merge-tree` or a dry-run rebase only to understand conflicts — don't resolve conflicts unless asked to fix the run, not merely investigate it.

## Check Live Processes

If `jrun.yaml` says `state: running`, verify the recorded process still exists and its start time matches:

```bash
ps -o pid,ppid,pgid,stat,etime,command -p <pid>
ps -axo pid,ppid,pgid,stat,etime,command | rg '<run_id>|<workspace_path>|<task_id>'
```

If asked to kill a run: match run id → task id(s) → `pid` → `pgid` → command; prefer process-group termination (`kill -TERM -<pgid>`, wait, verify with `ps ... | awk '$3==<pgid>'`); escalate to `kill -KILL -<pgid>` only if children remain and the human clearly asked to kill it; if the killed child belongs to a parent gate/auto run for the same task, inspect the parent and kill it only after verifying it owns the same task(s); report whether the run record updated to `failed`/`cancelled` or still says `running` despite no live process.

## Report Format

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

Before finalizing a diagnosis, verify: you matched the right run id and task id(s); you identified the first failed step, not just the last logged error; you checked recovery events when present; you checked stdout/stderr blobs for the failing invocation when available; you checked live process state for runs marked `running`; you separated root cause from downstream fallout; you filed friction (via `orbit-task`) if Orbit diagnostics or recovery behavior were misleading.
