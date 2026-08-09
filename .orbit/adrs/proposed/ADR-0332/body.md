## Context

The planning duel is unused, duplicates crew-based model selection, and owns thousands of lines across execution, persistence, tooling, configuration, CLI, dashboard, and documentation. Existing workspaces still contain the `[duel]` tables written by older `orbit init`, shipped databases contain a nullable invocation `slot` column, and task bundles may contain historical duel artifacts.

## Decision

Remove the planning-duel activities, job, runner, types, scoreboards, tools, CLI and dashboard surfaces, plus the duel-only per-dispatch model override and role-slot APIs. Continue accepting the retired `[duel]` and `[duel.models]` tables with a warning, leave the shipped nullable SQLite `slot` column in place while ceasing to write it, and leave historical task-bundle artifacts inert rather than migrating or deleting them.

## Consequences

- Agent dispatch selects provider and model only through activity assets and crew resolution.
- Scoreboard summary schema v8 drops duel projections; maintained dashboard, website sync, and README consumers tolerate the resulting shape.
- Existing initialized workspaces keep starting while operators receive explicit cleanup guidance.
- Historical database columns and task artifacts remain readable inert residue.
- Cost: the compatibility warning, frozen nullable SQLite column, and inert task artifacts remain until a future migration window explicitly retires them.