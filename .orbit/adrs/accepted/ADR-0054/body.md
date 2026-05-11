## Context
The runtime needs a crisp signal for "this attempt succeeded" versus "this attempt failed" without parsing freeform assistant text.

## Decision
Groundhog uses dedicated builtins for checkpoint success, checkpoint failure, and side-effect recording. The runner treats missing terminal verbs as synthetic failure.

## Consequences
- Attempt closure is deterministic and machine-readable.
- Retry logic does not depend on assistant prose conventions.
- Cost: the tool surface becomes load-bearing; mismatches between docs and registered builtins are high-risk drift.
