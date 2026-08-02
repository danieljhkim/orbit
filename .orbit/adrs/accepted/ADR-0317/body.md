## Context

Orbit records a job run's owner as a PID plus a process-start identity token, and the orphan sweep marks a `running` run `interrupted` when both `ps` and `kill(pid, 0)` say that PID is gone. A PID, however, only names a process *within a PID namespace*. Orbit's own sandbox (`bwrap --die-with-parent --new-session --unshare-all --share-net --dev /dev --proc /proc`) gives every sandboxed agent a private PID namespace and a fresh procfs. Inside it, host worker PIDs are invisible, so both probes answer confidently and wrongly: `process_not_found`. Sandboxed agents routinely invoke the Orbit CLI, and many CLI surfaces reconcile.

**Incident 2026-08-02.** `jrun-20260802-2013-2` (`task_pr_pipeline`) ran to a complete success — all 9 steps succeeded, provider PID 83502 exited 0 after ~42 min, PR opened at 20:55:49Z — yet its record read `state: interrupted`, `finished_at: 20:43:00.148Z`, `duration_ms: 1754138` (~29 min). Two sibling runs were condemned in the same pass, 13 ms apart. The condemning process was PID 40790 with cwd `.../worktrees/orbit-jrun-20260802-2013-2` — an agent inside that run's own sandbox. Its PID sequence (40345…40836, +6..10 per invocation) runs concurrently with the host sequence (190031…228874) in the same seconds: two independent monotonic PID allocations, i.e. two namespaces. The false state then cascaded — both parents failed `pipeline_success_guard` on the child's fabricated status.

Across history this is not rare: of 60 `reason=process_not_found` interrupts, **32 were false positives** (the run's audit trail shows activity after its recorded `finished_at`), clustering into synchronized batches — 3 at 20:43:00, 5 at 19:45:23, 6 at 05:07:34, 3 at 01:58:44, 3 at 00:09:14 — the signature of one cross-namespace sweep condemning every live run at once. 27 were genuine deaths, the capability that must be preserved.

**ORB-10557 already diagnosed this root cause** (2026-08-01) and shipped a gate: skip reconciliation when `ORBIT_MANAGED_RUN_CONTEXT` + `ORBIT_RUN_ID` mark a managed child. It did not hold, because the gate wraps exactly one call site — `reconcile_stale_job_runs_on_open` in `OrbitRuntime::from_roots`. Reconciliation has at least five other entry points that are ungated: `job_history`, `list_job_runs`, `show_job_run`, `execute_pipeline_run_worker`, and `release_stale_owned_task_reservations`. The 2026-08-02 sweep entered through the last of these: the audit shows `task.locks.reserve.released` from PID 40790 at 20:43:00.155, bracketing the three condemnations at .148/.154/.161.

## Decision

Make namespace scope a property of the *decision*, not of the entry point.

1. The process-start identity token is versioned up to `ps-lstart-utc-v2:pidns=<inode>:<lstart>`, recording the PID namespace (`/proc/self/ns/pid`) of the process that wrote it. v1 tokens are still read and still verify against a v2 probe on their process-start value, so an in-flight run claimed by an older binary is not invalidated by the upgrade.
2. Owner classification compares the observer's namespace against the recorded one *before* any probe runs. A mismatch yields a new terminal-safe classification, `OwnerIdentity::ForeignPidNamespace`, which joins `ProbeUnavailable` in the never-stale set and is tagged `reason=foreign_pid_namespace` in diagnostics. Cancellation refuses to signal across the boundary for the same reason.
3. A missing namespace on either side yields `Unknown`, which behaves exactly as before. This is deliberately asymmetric: the guard never converts "unknown" into "foreign".
4. An orphaned run's `finished_at` is derived from the last event in its own audit trail (clamped to `[started_at, now]`) rather than the moment of detection.

ORB-10557's env gate is left in place: it is a cheap first line of defense for the common case, and this ADR does not depend on it.

### Rejected alternatives

- *Extend the env gate to every reconciliation entry point.* Cheaper, but it is a per-call-site allowlist that must be re-audited whenever a new caller appears — which is precisely how ORB-10557 failed. It also trusts an inherited environment variable rather than an observable kernel fact.
- *Consult `provider_processes` for liveness.* The table did track PID 83502 correctly throughout. But it is an audit projection of the *provider child*, not the run owner, it is absent for non-CLI steps, and reading it cross-namespace has the identical PID-meaning problem.
- *Add a heartbeat or step-progress signal.* Genuinely useful and orthogonal, but it changes the write path of every step for a defect whose cause is a misread of an existing correct signal. Left for a follow-up.
- *Treat "observer is not in the initial PID namespace" as foreign for tokenless runs.* Rejected: it would disable genuine-orphan detection outright for any deployment whose workers legitimately run inside a container, and it would break detection for the common `pid_start_time: None` case (an owner whose `ps` probe failed at claim time).
- *Weaken `pipeline_success_guard` to tolerate `interrupted`.* Explicitly out of scope. The guard behaved correctly on false input; the input was the defect.

## Consequences

- A run whose owner was recorded in another PID namespace can no longer be condemned from that namespace, through *any* reconciliation entry point, including ones added later.
- Genuine orphan detection is unchanged when observer and owner share a namespace — the case that covers every host-side sweep, `orbit doctor`, and the dashboard.
- Recorded `duration_ms` for genuine orphans no longer includes detection lag, so cost and throughput metrics reading it stop over-counting.
- The false-interrupt vector that automatically released task-file reservations (`StaleRunReconciled`) while the owning run was still editing those files is closed at its source.
- `Cost:` the identity token gains a namespace field, so a v2 token written by a new binary is not byte-comparable with a v1 token written by an old one. Comparison is version-tolerant rather than string equality, which is one more rule for a reader of `process_identity.rs` to hold. A same-namespace PID reuse is still caught; a *cross*-namespace PID reuse is now reported as unverifiable instead of as a mismatch, which is the correct answer but a strictly weaker one.
- `Cost:` runs claimed by a pre-ORB-10594 binary carry no namespace field, so they remain condemnable from a foreign namespace until they finish. The exposure is bounded by run lifetime (hours), and closing it would require the rejected initial-namespace heuristic.
- `Cost:` an orphaned run with no audit trail still falls back to detection time for `finished_at`.