## Context

Task history was suspected of diluting downstream readers with machine-generated
bulk, but "history" was ambiguous across three candidate surfaces — orbit history
events, agent run transcripts, and task comments — and a mitigation aimed at the
wrong one would add a truncation path to maintain while leaving the real bulk in
place. So the surfaces were measured first, with a committed re-runnable method
(`scripts/measure-history-signal.py`).

Over 845 task bundles in the orbit workspace on 2026-08-09:

- `events.jsonl`: n=5,208 entries, 1,027,732 B. mean 197, p50 160, p95 213,
  **max 85,005**. Boilerplate (JSON envelope) ratio 0.755.
- `comments.jsonl`: n=1,581, 1,069,757 B. mean 677, p50 461, p95 1,662,
  max 26,723. Boilerplate ratio 0.158.

Every history entry above 2 KB was the same event type: `workflow_run_failed`.
Nine entries carried 170,993 B — **16.6% of all history bytes in 0.17% of
entries** — and all nine blob-shaped notes in the corpus were exactly those. The
cause is `workflow_failure_note` inlining a run's whole `error_message`; a
worktree-integrity failure serializes its entire `dirty_paths` list into that
field, which is how one ORB-10332 note reached 85 KB. The offender is in this
workspace, so no reroute applies. Comments are a fat middle (52 entries over
2 KB, but only 8 blob-shaped in 1,581) — human and agent prose, not machine
bulk, and not a truncation problem.

The decisive fact for the fix: `job_run_steps.error_message` persists the whole
message for the life of the run record. The 80,939-byte text behind that
ORB-10332 note is still there today. The history note was carrying a *duplicate*
of an already-durable value.

## Decision

Elide an oversized `error_message` from the `workflow_run_failed` history note,
keeping a leading excerpt and naming the retrieval command — `orbit run show
<run_id> --json`, field `.run.steps[].error_message` — **inside the note
itself**, so a reader who hits the elision does not have to know where run
records live.

The general rule this instantiates, now normative in
`docs/design/task-artifacts/specs/task-bundle-v2.md` §Events: a history note may
elide content only where another record retains it in full. Discarding a value
that exists nowhere else stays forbidden — `events.jsonl` is append-only and a
lossy write cannot be undone when someone later needs the detail.

The threshold is `MAX_NOTE_ERROR_BYTES = 1000`, declared exactly once in
`orbit-engine`'s `context::outcome`. It comes from the real distribution of 497
recorded step errors (p50 183 B, p95 676 B, p99 14,720 B, max 80,939 B): the p95
message stays inline verbatim and only 18 of 497 (3.6%) elide.

Two guards, because the failure mode is silent. `scripts/check-history-note-size.sh`
(wired into `make ci-fast` and `make ci`) fails on a second threshold
declaration, a second `workflow_run_failed` note producer, or an elision that
drops its retrieval pointer. `crates/orbit-engine/src/context/tests/outcome.rs`
pins the runtime bound, the verbatim pass-through below the cap, and UTF-8-safe
slicing of arbitrary subprocess bytes.

## Alternatives rejected

- **Spill the payload to the content-addressed blob store
  (`orbit-common::utility::blob_store`) and reference it by hash.** This is the
  shape the task anticipated, and it was rejected once measurement showed the
  full text is already durable in `job_run_steps`. Adding a blob write would
  create a second copy of an existing record, a second retention lifetime to
  reason about, and a retrieval path a reader has to be taught — for no
  recoverability the run record does not already give.
- **Cap every history note at the store write boundary.** One place, but it
  would truncate notes whose content is *not* recoverable elsewhere, converting
  a general dilution problem into a general data-loss problem.
- **Leave it and filter on read.** Every reader would need the filter, and the
  bytes stay in an append-only file forever.

## Consequences

- The real ORB-10332 note goes from 81,031 B to 1,221 B (98.5%); its `events.jsonl`
  row from 85,020 B to 1,466 B. Applied to the nine measured entries, ~16% of all
  task history bytes in the workspace stop being written.
- Agent task context shrinks with it: `v2_host/task_context.rs` injects the most
  recent `workflow_run_failed` note into the implementer's context verbatim, so
  the 81 KB blob was being paid for again on every dispatch against a
  previously-failed task.
- The retained excerpt still names the failure and the branch, which was the one
  fact a reader was paying multiple KB to learn.
- Cost: diagnosing an elided failure now needs a second command against the run
  record. The command is printed in the note, but a reader working from an
  exported or copied history string, without the run store to hand, has less
  than before.
- Cost: a third guard script in the CI chain, and a threshold whose value is only
  justified against one workspace's distribution. Re-run
  `scripts/measure-history-signal.py` before changing it.
- Not addressed, and deliberately: history's 0.755 boilerplate ratio (776 KB of
  1,028 KB is JSON envelope — ids, timestamps, statuses). That is structural to
  the append-only row format, not low-signal content, and reducing it would be a
  bundle-format change rather than a writer change.