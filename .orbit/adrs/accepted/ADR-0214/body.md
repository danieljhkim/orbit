## Context

A parent pipeline run reaching terminal `interrupted` (crash, reboot) could strand its queued child runs in `pending` forever: pending runs recorded no owner process, `reconcile_stale_job_runs_on_open` only handled orphaned `running` runs, `orbit doctor` only reported running orphans, and no CLI could terminalize a stuck run. Two four-day-old pending gate runs were observed in `codebases/sextant`, demonstrating that stale queue state could persist indefinitely.

## Decision

Give queued runs the same owner-liveness contract as running runs. The pipeline worker claims its run at startup (`claim_pending_job_run_owner` records `pid` + start-time token while the run is still `pending`). Reconcile finalizes a pending run as `interrupted` only when the claimed owner is Mismatch/Missing, or when the run was never claimed and is older than a 30-minute grace window. Inconclusive probes and fresh unclaimed runs stay pending. `orbit doctor` reports both orphan classes, and `orbit run cancel <run_id>` is the manual path.

## Consequences

- Orphaned pending runs self-heal at workspace open and lazily on run list/show.
- Queued runs written by pre-claim binaries whose worker is still alive are shielded only by the grace window; a live legacy queued run older than 30 minutes may be terminalized once, after which its worker exits cleanly and the run remains resumable.
- `pid` on a pending run means claiming worker; `mark_job_run_running` overwrites it when execution starts.
- Cost: the grace-window heuristic can interrupt a still-live legacy queued run because old binaries never record ownership.