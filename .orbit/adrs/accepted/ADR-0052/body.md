## Context
Groundhog needs a durable, machine-readable checkpoint list. Freeform task plans do not give the runner enough structure to decide what to retry, verify, or record.

## Decision
Groundhog reads typed checkpoints from the task's structured `plan` field.

## Consequences
- Checkpoint identity, success criteria, and retry budget are available to both runtime and agent.
- The task artifact becomes the authoritative source of execution structure.
- Cost: Groundhog inherits the quality of the task plan; weak checkpointing produces weak execution.
