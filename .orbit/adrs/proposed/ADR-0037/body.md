## Context
Friction reports once used a dedicated task type, but untriaged reports shared `status: proposed` with human-authored proposals, making scoreboard derivation ambiguous.

## Decision
Add `status: friction` as the creation status for self-reports, infer legacy friction routing at creation, and rebuild `friction_bounty.json` from task history.

## Consequences
- Friction inbox items are separated from human proposals while legacy friction task records remain readable.
- Cost: legacy untriaged reports need migration, and already-triaged legacy histories depend on existing transition records.
