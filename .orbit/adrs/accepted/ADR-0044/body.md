## Context
Local Orbit task review threads and GitHub PR review comments are different workflow artifacts, and reply volume should not be scored as distinct review findings.

## Decision
Keep `pr.review_comments` for synced PR/GitHub comments, score local review-thread creations separately as `task-review-threads` surfaced as `task_review.threads`, do not score replies, and accept only exact configured or built-in model identities.

## Consequences
- Local review feedback earns immediate task-review credit while synced PR feedback remains a separate PR metric.
- Cost: review productivity now has two counters, and aggregate views must label them clearly rather than adding them blindly.
