## Context
Operators need first-class commands for activity/job envelope JSONL, but `orbit audit` is the compact SQLite command-audit surface.

## Decision
Expose v2 envelope inspection under `orbit run events` and `orbit run trace`, and keep envelope/blob parsing behind orbit-core runtime accessors.

## Consequences
- Command history and run-local workflow traces have dedicated commands.
- Cost: users must learn that `orbit audit` and `orbit run events/trace` answer related but different questions.
