## Context
Task-shipping PRs now carry one task, but the default generated body still reflected the older batch shape. Reviewers had to leave the PR to read the task description and acceptance criteria, while GitHub already rendered the changed-file list natively.

## Decision
Render one-task PR bodies as `## Task`, optional collapsed `## Execution Summary`, `## Validation`, and `## Branch Freshness`. The task section includes the task link, verbatim description, and plain-bullet acceptance criteria. Multi-task callers keep the legacy body while those paths remain supported.

## Consequences
- Cost: _(migration: missing in source)_
