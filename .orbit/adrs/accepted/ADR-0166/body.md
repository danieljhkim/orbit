## Context
The scoreboard started as one compact per-agent table. Adding knowledge-artifact counters and planning-duel matrix data made the flat table mix delivery attribution, review work, operations, knowledge stewardship, and duel outcomes in one scan path. Alternatives were to keep widening the table, add column groups inside the same table, or split the view into focused sections.

## Decision
Render the dashboard scoreboard as focused sections: Delivery, Review, Knowledge, Operations, Planning Duels, a family-vs-family Duel Matrix, and Attribution Cleanup for non-canonical rows. Keep compact pair cells where they still help local interpretation, but do not treat the whole scoreboard as one primary flat leaderboard.

## Consequences
- Operators can inspect one contribution dimension at a time without conflating task creation, planning, implementation, review, tool usage, and knowledge artifacts.
- Non-canonical attribution rows stay visible but no longer compete with canonical agent families in primary sections.
- No single Rust code anchor; this is enforced by dashboard rendering and design review, and workspace-local ADR comments should not be embedded in shipped dashboard assets.
- Cost: Cross-section comparison now requires scanning multiple tables instead of one row, and future metrics must choose an explicit section before being added.