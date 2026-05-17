## Context
Design docs were enforced by contributor convention and a Python make target, while ADRs and tasks already had first-class CLI and MCP surfaces. The real alternatives were to keep design-doc checks as repo-local scripts, to add independent CLI and MCP implementations, or to make both surfaces share one core implementation.

## Decision
Orbit will expose design docs through `orbit design check` and the `orbit.design.*` tool namespace, with shared behavior owned in `orbit-core::command::design`. The legacy Python checker remains only as a compatibility delegator to `orbit design check` instead of preserving a second implementation.

## Consequences
- CLI, MCP, and workspace initialization all consume the same design-doc conventions and decay logic.
- Design-doc tooling can be promoted as a primary feature because users and agents can invoke it directly through Orbit.
- Cost: The design-doc checker now depends on the Orbit binary being available for the compatibility script and make target path, so development and packaging must keep the binary command path healthy.