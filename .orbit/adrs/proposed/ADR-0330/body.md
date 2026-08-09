## Context

ADR-0213 flattened a crew to one provider-model-backend assignment but kept two compatibility surfaces: legacy planner/implementer/reviewer sub-tables accepted at config load, and an AgentRole label carried on activities and job steps. Both became inert: legacy load kept one assignment while discarding the others, and every role resolved to the same run crew. The alternative was to keep these shims indefinitely, at the cost of schemas that advertised selection they did not perform.

This amends the legacy-config and role-label clauses of ADR-0213. Its core decision — one assignment per crew — stands and is completed here.

## Decision

Crew configuration accepts flat provider/model/backend fields only; legacy role sub-tables are rejected with rewrite guidance. AgentRole and role-to-assignment resolution are removed from config, runtime, and asset schemas. A rendered activity input with a non-empty crew selects that named crew. An activity without one dispatches on the resolved run crew, replacing the inline-baseline fallback.

## Consequences

- Activity routing has one mechanism: an input names a non-default crew or dispatch inherits the run crew.
- A heterogeneous legacy crew fails loudly instead of silently losing assignments.
- System activities that need a different model name a configured crew through rendered input.
- Cost: this is a breaking config and asset-schema change; legacy crew tables and role-bearing assets must be rewritten before they load.
- Cost: an activity-specific crew choice lives in rendered job input, so authors must inspect the referring job as well as the activity asset when auditing an intentional non-default route.