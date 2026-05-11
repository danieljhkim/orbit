## Context
Groundhog has its own state, retry loop, and checkpoint-closing builtins. Treating it as an `agent_loop` toggle would have hidden that behavior inside flags and made dispatch harder to reason about.

## Decision
Groundhog is its own `ActivityV2Spec::Groundhog` variant with a dedicated runner.

## Consequences
- Dispatch can validate Groundhog-specific preconditions up front.
- Runtime code gets a clear place to own checkpoint state and snapshot handling.
- Cost: one more activity shape to document, validate, and keep aligned with `agent_loop`.
