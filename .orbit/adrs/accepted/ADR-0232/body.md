## Context
Bridge parity duplicated Orbit schemas, errors, and workflow declarations despite Orbit being the canonical domain owner.
## Decision
Retire Bridge’s Orbit-shaped contract after Orbit MCP reaches parity; Bridge remains only for its non-Orbit constellation domains.
## Consequences
- Clients register Orbit and Bridge side by side during migration.
- Cost: cutover temporarily maintains two client registrations and requires deleting a compatibility layer rather than extending it.