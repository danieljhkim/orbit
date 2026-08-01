**Context.** A recovered rebase previously retried the composite `pr_open` action, replaying commit and remote side effects. The alternatives were to make that composite infer prior work from commit subjects and generic divergence, or to expose each handoff phase as durable job state with explicit rewrite provenance.

**Decision.** Model commit, pre-rewrite branch preparation, exact-base rebase, push, PR create-or-reuse, and task promotion as separate `task_pr_pipeline` activity steps. A divergent push is authorized only when a persisted preparation checkpoint names the exact remote SHA observed before the rewrite, the rebase phase confirms that a rewrite occurred, and the push uses a branch-scoped `--force-with-lease=<ref>:<sha>`; all ambiguous or changed remote state fails closed.

**Consequences.**
- Recovery resumes at the first incomplete job step, while step output records whether each phase was performed, skipped, or reused.
- Remote-only commits are never treated as implicit authorization to force-push, and PR retries discover the branch PR before creating one.
- Cost: the shipped workflow and deterministic activity catalog gain three focused activities plus explicit data plumbing between their output schemas.
- Cost: push performs remote inspection/fetch work before classifying non-current refs, and operators must reconcile ambiguous divergence manually.