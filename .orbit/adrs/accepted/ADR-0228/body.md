## Context
A single remote MCP target must preserve local graph/doc behavior and must not equate tool placement with caller privilege.
## Decision
Make the client-facing MCP process a local placement broker whose canonical tools declare hub, owner, local-derived, or composite placement and whose effective capability set is independently filtered.
## Consequences
- Tool schemas have one placement and a non-empty allowed capability set.
- Cost: the broker owns route preflight, composite auditing, and capability-by-placement conformance coverage.