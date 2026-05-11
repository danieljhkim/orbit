## Context
Asking agents to pass both `agent` and `model` duplicated information and allowed exact models to be paired with the wrong family.

## Decision
Deprecate `agent` as a normal tool-call input, prefer exact `model`, infer the agent family from known model names, and reject inconsistent legacy pairs.

## Consequences
- Seeded skills and instructions can use shorter model-only tool calls while task records still retain both fields internally.
- Cost: unknown or ambiguous models still need a compatible legacy `agent` value when family-specific dispatch matters.
