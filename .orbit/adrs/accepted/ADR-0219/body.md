## Context

Some normal workflow tasks produce durable side effects through Orbit rather than repository changes. QA validation files follow-up tasks and may correctly leave the worktree unchanged; treating every empty diff as an implementation failure strands valid runs, while weakening the gate globally would hide broken implementation tasks.

## Decision

`no-diff-expected` is a first-class task tag. The commit and PR handoff gates bypass empty-stage and zero-commits-ahead failures only when the relevant task (or every task in a PR bundle) carries the tag; the run still requires a meaningful execution summary and advances through the normal lifecycle without creating an empty commit or PR.

## Consequences

- Side-effect-only validation tasks can complete through normal orchestrator dispatch.
- Ordinary implementation tasks retain fail-closed empty-diff checks.
- The checked-in QA auto-task template carries the exemption explicitly, keeping the exception visible in data.
- Cost: a mistagged task can reach review without repository changes, so definition authors and reviewers must treat this tag as a privileged workflow exemption.