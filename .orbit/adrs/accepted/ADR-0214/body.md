## Context

A parent pipeline run reaching terminal `interrupted` (crash, reboot) could strand its queued child runs in `pending` forever: pending runs recorded no owner process, `reconcile_stale_job_runs_on_open` only handled orphaned `running` runs, `orbit doctor` only reported running orphans, and no CLI could terminalize a stuck run. A single stale `pending` run permanently deferred `deploy/orbit-web-upgrade.sh` (observed live 2026-07-10 in `codebases/sextant`: two 4-day-old pending gate runs blocked the daily binary swap).

## Decision

Give queued runs the same owner-liveness contract as running runs. The pipeline worker claims its run at startup (`claim_pending_job_run_owner` records `pid` + start-time token while the run is still `pending`). Reconcile finalizes a pending run as `interrupted` (`Pending + Interrupt` becomes a legal transition) only in two conclusive cases: the claimed owner is Mismatch/Missing, or the run was never claimed and is older than a 30-minute grace window. Inconclusive probes and fresh unclaimed runs stay pending. `orbit doctor` reports both orphan classes; `orbit run cancel <run_id>` is the manual path; the upgrade script's deferral gate checks worker liveness (recorded pid, else a process holding the run id on its cmdline) instead of trusting `running|pending` state, so it self-heals even under a pre-fix installed binary.

## Alternatives

- Reconcile by parent linkage (terminal parent implies orphaned children): rejected — no parent run id is persisted on child runs, and it would miss stale pendings whose parent succeeded or that have no parent.
- Age-only heuristic for all pendings: rejected — queued runs legitimately wait hours behind `max_active_runs`; age alone would terminalize healthy queued runs. The grace window applies only to runs with no recorded owner, a state that post-claim binaries leave within seconds.
- Using the freshly built binary for the upgrade script's run-history probe: rejected — the new binary auto-applies workspace migrations on open before the swap decision, which could leave workspaces ahead of a still-installed old binary when the upgrade defers.

## Consequences

- Orphaned pending runs self-heal at workspace open and lazily on run list/show; the daily upgrade can no longer be blocked by dead state.
- Queued runs written by pre-claim binaries whose worker is still alive are shielded only by the grace window plus a claimed-owner check that never fires for them; a live legacy queued run older than 30 minutes would be terminalized once, at upgrade time. Its worker exits cleanly on observing the terminal state, and the run is resumable via `orbit job resume`.
- `pid` on a `pending` run now means "claiming worker", not "executing worker"; `mark_job_run_running` overwrites it with the same process id when execution starts.