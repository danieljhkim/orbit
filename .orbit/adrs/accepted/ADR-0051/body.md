## Context
Groundhog relies on a fresh prompt boundary per attempt and on explicit builtin closures. The existing CLI-backend path does not expose the same runtime control surface.

## Decision
Groundhog's shipped runner is HTTP-only. Dispatch rejects providers whose HTTP transport is not wired.

## Consequences
- The first ship stays inside the transport model the runtime already controls.
- The provider/type surface remains narrower in practice than the enum implies.
- Cost: CLI-backed execution gets no Groundhog behavior unless the transport story broadens later.
