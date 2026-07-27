## Context

`WorktreeIntegrityGuard::verify` compares the registered primary checkout before and after a provider invocation. Until now it had exactly one benign case: `primary_fast_forward_is_benign`, which accepts a proven same-branch fast-forward whose dirt does not intersect `run_changed_paths` (ORB-10471). That helper rejects `before.head == after.head` on its first clause, so a primary that never moved but merely gained or lost dirt was always fatal.

F2026-07-166 is the cost: run `jrun-20260726-2223-8` (ORB-10467) lost a complete, validated 13-file implementation because an out-of-run learning-curation pass re-serialized 12 already-tracked `.orbit/learnings/*/learning.yaml` files in the primary while its HEAD and branch stood still. `conflicting_paths` was empty; the guard raised a non-retryable `primary_checkout_drift` anyway and burned the step's entire `max_attempts=1` budget. This dirt class is endemic, not exceptional: Orbit's own engine and curation passes rewrite the primary's `.orbit/` records continuously while pipelines run.

The obvious fix — reuse the fast-forward rule verbatim, accepting any stationary-HEAD delta disjoint from `run_changed_paths` — is unsafe, and the existing suite proves it. Five tests (`primary_escape_is_typed_non_retryable_and_preserves_both_checkouts`, `primary_content_mutation_is_typed_even_when_assigned_content_also_changes`, `staged_only_primary_delta_reports_its_path_and_index_identity`, `unchanged_pre_dirty_path_is_excluded_from_escape_diagnostic`, `primary_escape_is_checked_after_nonzero_exit_and_timeout`) encode the ORB-10134 data-loss hazard: a provider that escapes its assigned worktree and edits the primary. Every one of those escapes is disjoint from the run's own paths — `primary_content_mutation_is_typed_even_when_assigned_content_also_changes` is named for exactly that shape — so a pure disjointness rule silently accepts all five.

The asymmetry is evidential. A fast-forward carries positive proof that Git itself produced the delta (`merge-base --is-ancestor` over a moved HEAD). A stationary HEAD carries no such proof: at the Git layer, a curation pass rewriting a record and a provider stray-writing a source file are indistinguishable by authorship. Only the *path class* separates them.

## Decision

Add a second benign case, `primary_dirt_only_delta_is_benign`, that is deliberately narrower than the fast-forward rule. A stationary-HEAD primary delta is accepted only when all of the following hold:

1. `before.head == after.head` and `before.branch == after.branch`;
2. the delta is fully explained by dirt-path movement (`primary_dirt_mutations` is non-empty) — an unattributable fingerprint change falls through and fails closed;
3. no mutated path intersects `run_changed_paths` (the same gate ORB-10471 introduced);
4. **every** mutated path lives under `.orbit/`, Orbit's own record store.

Clause 4 is the load-bearing addition. `.orbit/` holds tasks, ADRs, learnings, frictions, and routines: content the engine driving the pipeline rewrites as a matter of course, and content that is never a run's code candidate. A primary mutation anywhere else — source, manifests, CI config — remains `primary_checkout_drift` regardless of disjointness, preserving ORB-10134 escape detection intact.

Acceptance is logged at `info` on `orbit.engine.cli_runner` with the ignored paths, matching the fast-forward case. The guard never cleans or reconciles the dirt it ignores.

## Consequences

- The F2026-07-166 class of loss is closed: concurrent record-store curation can no longer strand a validated implementation.
- Provider-escape detection is unchanged. All five ORB-10134 escape tests still fail closed, and `stationary_primary_source_edit_stays_fail_closed_even_when_disjoint` pins the divergence so a future "simplification" to pure disjointness fails loudly.
- The two benign cases now apply asymmetric rules, which is a genuine complexity cost to carry.
- **Cost:** clause 4 is a path-prefix heuristic, not a proof of authorship. A benign out-of-run pass that touches a primary file *outside* `.orbit/` still fails the guard, and a provider that escapes to write inside the primary's `.orbit/` is now tolerated. Both are chosen deliberately: the first errs fail-closed on the guard's own hazard, and the second is a records-not-code blast radius that the run's own record writes already produce. If either turns out to matter, the follow-up is authorship evidence (e.g. fingerprinting per-path mtime or an engine-owned write ledger), not a wider prefix list.
- **Cost:** this narrows the literal fix ORB-10493 proposed ("compare only the dirt paths that intersect `run_changed_paths`"). Its first acceptance criterion is met for the friction's actual repro, which is a record-store delta, but not for an arbitrary disjoint tracked file. Rejected alternative: implement the criterion literally and rewrite the five escape tests — that trades an intermittent, recoverable failure for a silent data-loss regression, which is the wrong direction on this guard.