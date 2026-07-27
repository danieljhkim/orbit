## Context
Git-committed routines converge to many checkouts, while scheduler cursors and pauses must remain locally meaningful.
## Decision
A committed routine is owned by its registry-validated host pin; unpinned committed routines fail closed, and each host retains its own cursor and pause state.
## Consequences
- Reassigning a routine is a reviewed pin change rather than a git-status inference.
- Cost: handoff starts with no migrated cursor and existing committed routines need explicit pins before enforcement.