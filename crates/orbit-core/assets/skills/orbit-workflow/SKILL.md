---
name: orbit-workflow
description: How to use Orbit's execution layer — jobs, activities, routines, `orbit sweep`, and `orbit run` — and how to diagnose a failed, stuck, cancelled, or suspicious job run. Triggers on running/inspecting a job or pipeline, scheduling routines, or when a human provides a `jrun-*` id, says a job failed, asks why a run is stuck, which task a run is handling, whether to kill a run, or wants a failure diagnosis involving Orbit activities/jobs.
---

# Orbit Workflow

## Concepts

- **Job** — a deterministic, multi-step pipeline (schemaVersion 2 YAML) under `crates/orbit-core/assets/jobs/` (e.g. `task_pr_pipeline`, `task_auto_pipeline`, `task_gate_pipeline`, `task_epic_pipeline`, `job_duel_plan_pipeline`). Jobs compose **activities**.
- **Activity** — one named step definition under `crates/orbit-core/assets/activities/*.yaml` (`agent_implement`, `agent_review`, `git_commit`, `git_push`, `pr_open`, `worktree_setup`, `reserve_locks`, ...). Activities aren't invoked directly by CLI; a job's step list references them.
- **Routine** — a git-versioned cron trigger (`.orbit/routines/*.yaml`) pointing at a `job:<name>` target, with host pinning and a retry/overlap policy.
- **Sweep** — the stateless per-minute clock tick (`orbit sweep`, fired by an OS timer) that fires whatever routine is due on this host. All scheduler state (last fires, pauses, locks) is host-local in `~/.orbit/orbit.db`, never synced; routine *definitions* sync via git.
- **`orbit run`** — the execution frontend: `orbit run ship` / `ship-local` / `ship-sweep` (dispatch across every registered workspace) / `duel-plan` / `job <id>` / `history` / `show <run_id>` / `logs <run_id>` / `events <run_id>` / `trace <run_id>`. Equivalent job-catalog commands exist as `orbit job list|show|run|replay|resume`.

## Running a job

```bash
orbit job list                                  # catalog
orbit job show <job_id>
orbit run job <job_id> --input key=value --json  # or: orbit job run <job_id> --input key=value
orbit run ship                                   # ship backlog/selected tasks through the gated pipeline
orbit run history --json
orbit run show <run_id> --json
```

## Routines & scheduling

1. **Host identity** — `orbit routine init --host-id <id>` (defaults to hostname); add `--install-clock` to install the per-user OS clock unit that runs `orbit sweep` every minute.
2. **Mark a workspace a routine source** — `[routines]` / `role = "source"` in that workspace's `.orbit/config.toml` (any other `role` value is a fail-closed config error).
3. **Add a routine** — YAML under `<source>/.orbit/routines/`:
   ```yaml
   schemaVersion: 1
   name: almanac-auto-commit
   enabled: true
   hosts: [dk-mac]                      # explicit pinning; no "any host" in v1
   trigger: { cron: "0 22 * * *", missed_run: skip }   # skip | catch_up_once
   target: job:almanac_commit_pipeline  # job:<name> only — activity: is rejected
   policy: { timeout_minutes: 10, retries: { max: 2, backoff_minutes: 2 }, overlap: forbid }
   ```
   Parsing is fail-closed: an invalid file makes *that routine* absent and reports a load error — it never fires with defaults.
4. **Verify without firing** — `orbit routine list` (toggles + next-due), `orbit sweep --dry-run`, `orbit routine show <name>` (fire history).

Fires appear in `orbit run history` under actor `routine/<name>`. `orbit routine pause|resume <name>` is host-local and durable across reboots. Toggle resolution order when something doesn't fire: `enabled: false` (versioned) → host not in `hosts` (versioned) → local pause (this host). `orbit routine list` shows all three.

Read the full schema at `docs/design/routines/2_design.md` §1 before hand-authoring a routine — don't answer field semantics from memory.

## Checking Operational Logs

For a host-level incident, service warning, JSONL tracing problem, or missing
run output, use [references/operational-logs.md](references/operational-logs.md).
It separates journal/service logs, global JSONL tracing, and per-run evidence
and keeps runtime state read-only during diagnosis.

## Diagnosing a failed/stuck/cancelled run

Given a `jrun-*` id, see [references/debug-job-failure.md](references/debug-job-failure.md) for the full investigation flow (run bundle, v2 audit trail, logs/blobs, failure classification, task/git/process state, kill procedure, report format). Don't use it for ordinary task implementation unless the request is specifically about a failed run.

**Safety, up front:**
- Do not edit files under `.orbit/state/job-runs/` or `.orbit/state/audit/` to "fix" a run — treat them as evidence.
- Do not kill a process until you've matched run id → `pid`/`pgid`/task id(s)/command; prefer terminating the process group for that run id only.
- Do not kill parent auto/gate runs unless you've verified they own the same task(s).
- Do not rely on top-level `state: failed` alone — find the first failed step/activity.
- Do not parse agent prose as the durable handoff when task state, run state, or `orbit.state.*` records exist.
- If Orbit tooling or diagnostics are misleading, file friction via `orbit-task`.
