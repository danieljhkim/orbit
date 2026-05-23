## Context
The metrics CLI surface is unused, and ORB-00191 moved the missing knowledge, activity, tool, task, and invocation views into dashboard HTTP endpoints. Keeping a second JSON-capable command would make future metrics work maintain two surfaces.

## Decision
The dashboard is the canonical user-facing and programmatic surface for invocation metrics. The metrics CLI command is retired, and future observability features should ship as dashboard endpoints and views.

## Consequences
- Programmatic consumers use the dashboard HTTP API (`/api/metrics/*`) instead of a dedicated CLI JSON scripting surface.
- Future invocation-metrics features build as dashboard endpoints first.
- No single code anchor; this convention is enforced through design docs and review.
- Cost: shell scripts cannot rely on a dedicated metrics command and must call the local dashboard API or shared runtime libraries.