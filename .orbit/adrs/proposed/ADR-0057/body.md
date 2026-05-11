## Context
Earlier Groundhog drafts centered deviation stacks and retry critics. The current v1 goal is narrower: prove checkpoint + rewind + verifier + success-memory before reopening more complex control flow.

## Decision
Keep executor-authored deviation and critic-on-retry out of Groundhog v1. Revisit them only after the simpler loop has operational data behind it.

## Consequences
- The first shipped contract stays smaller and easier to reason about.
- Failure pressure shifts back to plan quality and explicit blocked outcomes.
- Cost: the current code still carries deviation-era leftovers, and v1 loses one potential escape hatch for bad plans.
