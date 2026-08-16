## Context

The opt-in post-publication child review made ship submission depend on deployed YAML shape preflight, review-only lineage, and duplicated contracts across four submission adapters. The subsystem added a second orchestration path whose reliability cost outweighed its isolated use. Keeping it dormant would preserve stale operator-facing inputs and hard-coded asset contracts.

## Decision

Remove the independent review activity, guard, child job, shipment inputs, lineage policy, deduplication branch, and deployed-asset contract preflight. Ship submission inserts the selected shipment run directly after the shared in-flight guard. Retain the generic invoke-and-wait activity, pipeline success guard, response-envelope support, and explicit crew resolution for their remaining consumers. Historical task comments remain opaque durable comments.

## Consequences

- Ship has one submission contract across CLI, dashboard, MCP tool, and deterministic action.
- Deployed workflow YAML is no longer loaded and shape-checked before run insertion.
- Existing seeded copies of retired workflow assets must be overwritten or removed during operator upgrade.
- Published execution profiles must be regenerated because the ship-closure digest changes.
- Cost: Orbit no longer offers a built-in opt-in post-publication review gate; teams needing one must compose a separate workflow outside ship.