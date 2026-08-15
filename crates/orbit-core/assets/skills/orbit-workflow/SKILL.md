---
name: orbit-workflow
description: Orbit's execution layer — jobs, activities, routines, `orbit sweep`, and `orbit run` — plus task-pilot preflight and diagnosing a failed, stuck, or cancelled run. Triggers on task-pilot, running or inspecting a job or pipeline, scheduling routines, or a `jrun-*` id.
---

# Orbit Workflow

## Concepts

- **Job** — a deterministic, multi-step pipeline (schemaVersion 2 YAML) from the installed job catalog; discover it with `orbit job list` / `orbit job show <id>` (e.g. `task_pr_pipeline`, `task_auto_pipeline`, `task_gate_pipeline`). Jobs compose **activities**.
- **Activity** — one named step definition referenced by a job's step list (`agent_implement`, `task_pilot`, `git_commit`, `git_push`, `pr_open`, `worktree_setup`, `reserve_locks`, ...). Activities aren't invoked directly by CLI; a job's step list references them.
- **Routine** — a git-versioned cron trigger (`.orbit/routines/*.yaml`) pointing at a `job:<name>` target, with host pinning and a retry/overlap policy.
- **Sweep** — the stateless per-minute clock tick (`orbit sweep`, fired by an OS timer) that fires whatever routine is due on this host. All scheduler state (last fires, pauses, locks) is host-local in `~/.orbit/orbit.db`, never synced; routine *definitions* sync via git.
- **`orbit run`** — the execution frontend: `orbit run ship` / `ship-local` / `ship-sweep` (dispatch across every registered workspace) / `job <id>` / `history` / `show <run_id>` / `logs <run_id>` / `events <run_id>` / `trace <run_id>`. Equivalent job-catalog commands exist as `orbit job list|show|run|replay|resume`.

## Running a job

```bash
orbit job list                                  # catalog
orbit job show <job_id>
orbit run job <job_id> --input key=value --json  # or: orbit job run <job_id> --input key=value
orbit run job <job_id> --wait                    # block until terminal; nonzero exit unless it succeeded
orbit run ship                                   # ship backlog/selected tasks through the gated pipeline
orbit run history --json
orbit run show <run_id> --json
```

### Task-pilot pipeline

When an orchestrator needs task selectors prepared before ship traffic, inspect
the job with `orbit job show task_pilot_pipeline`, then invoke it with:

```bash
orbit run job task_pilot_pipeline
```

The run is submitted to a detached worker and the command returns as soon as
the run is durable — it prints the run id and the inspection commands, and does
not claim the eventual outcome. Add `--wait` to block on the submitted run and
exit nonzero unless it succeeded.

With no input, the job discovers only `proposed`/`backlog` tasks in the
invoking workspace whose `context_files` is empty. To audit specific tasks,
pass their IDs explicitly; this mode audits exactly those tasks, including
tasks that already have selectors:

```bash
orbit run job task_pilot_pipeline --input task_ids=<TASK_ID>,<TASK_ID>
```

Use this job before reservation or conflict checks when ship traffic is high.
Do not fill `context_files` inline: the job's apply step writes only validated
selectors, reducing file-collision risk. An enabled workspace routine may
already run the zero-input job on a schedule; invoke an extra run when needed.

## Routines & scheduling

1. **Host identity** — `orbit routine init --host-id <id>` (defaults to hostname); add `--install-clock` to install the per-user OS clock unit that runs `orbit sweep` every minute.
2. **Mark a workspace a routine source** — `[routines]` / `role = "source"` in that workspace's `.orbit/config.toml` (any other `role` value is a fail-closed config error).
3. **Add a routine** — YAML under `<source>/.orbit/routines/`:
   ```yaml
   schemaVersion: 1
   name: <routine-name>
   enabled: true
   hosts: [<host-id>]                   # explicit pinning; no "any host" in v1
   trigger: { cron: "0 22 * * *", missed_run: skip }   # skip | catch_up_once
   target: job:<job-name>               # job:<name> only — activity: is rejected
   policy: { timeout_minutes: 10, retries: { max: 2, backoff_minutes: 2 }, overlap: forbid }
   ```
   Parsing is fail-closed: an invalid file makes *that routine* absent and reports a load error — it never fires with defaults.
4. **Verify without firing** — `orbit routine list` (toggles + next-due), `orbit sweep --dry-run`, `orbit routine show <name>` (fire history).

Fires appear in `orbit run history` under actor `routine/<name>`. `orbit routine pause|resume <name>` is host-local and durable across reboots. Toggle resolution order when something doesn't fire: `enabled: false` (versioned) → host not in `hosts` (versioned) → local pause (this host). `orbit routine list` shows all three.

Consult `orbit routine --help` for the full field schema before hand-authoring a routine — don't answer field semantics from memory.

## Diagnosis

- Host-level incident, service warning, JSONL tracing problem, or missing run output → [references/operational-logs.md](references/operational-logs.md).
- A `jrun-*` id that failed, stuck, or was cancelled → [references/debug-job-failure.md](references/debug-job-failure.md) for the full flow: run bundle, v2 audit trail, logs and blobs, failure classification, task/git/process state, kill procedure, report format.

**Safety, up front:**

- Files under `.orbit/state/job-runs/` and `.orbit/state/audit/` are evidence. Never edit them to "fix" a run.
- Never kill a process before matching run id → `pid`/`pgid`/task id(s)/command; terminate the process group for that run id only. Never kill a parent auto or gate run without verifying it owns the same task(s).
- Top-level `state: failed` is not a diagnosis — find the first failed step or activity.
- Task state, run state, and `orbit.state.*` records are the durable handoff. Never parse agent prose in their place.
- If Orbit's own tooling or diagnostics mislead you, file friction (`orbit-task`).
