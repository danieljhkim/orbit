## Context
 Once verbs are data, the obvious next step is to make the rest of
each surface data too: CLI output formatting and dashboard REST routes were both
candidates. Both would have grown the registry and both were rejected during the
friction pilot.


## Decision
 The spec declares *which* rendering a verb wants (`CliRender`) but
not how to render; the friction record and table printers stay in `orbit-cli` and
know friction field names. Dashboard route shapes, serde request bodies, and
HTTP-specific defaults stay hand-written in `orbit-dashboard`, which takes only
tool names and parameter names from the registry.


## Consequences


- The registry stays a description of *operations*, not of presentation, which
  keeps it readable and keeps its blast radius to contract.
- A REST path remains an interface design decision made per route, not a
  mechanical consequence of adding a verb.
- Adding a friction verb that should be reachable over HTTP is still a two-place
  change (registry + route).
- A noun whose output has a genuinely new shape needs a new `CliRender` variant
  plus a renderer — also two places.
- Cost: the dashboard is only partially derived, so it remains possible to add a
  verb and forget the route entirely; nothing fails, the verb is simply absent
  from the web UI, and no test catches it.