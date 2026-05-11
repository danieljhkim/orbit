## Context
The v2 job executor concentrated step dispatch, retry/recovery, construct orchestration, template rendering, validation, audit projection, and inline tests in one 2.8k-line file.

## Decision
Keep the public job-executor API stable, but organize the implementation as `job_executor/` child modules with `mod.rs` holding the exported entrypoints and private helpers shared through module-scoped visibility.

## Consequences
- Reviewers can inspect retry/recovery, target dispatch, fan-out, loop, validation, and audit behavior in smaller files without changing runtime semantics.
- The split preserves the existing engine/core and CLI-runner boundaries; no new crate edge or provider type crosses the activity/job layer.
- Cost: private helper movement now requires maintaining intra-module visibility and imports across several files instead of one lexical scope.
