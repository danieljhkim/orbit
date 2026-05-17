## Context

Most ADR practices are append-permissive: any decision can become an ADR, and pruning is after-the-fact. Append-permissive logs grow fast and bury load-bearing decisions in trivia ("we picked this library version," "we used `?` instead of `match`"). The question was whether to inherit the permissive default or gate entry.

## Decision

A decision earns an ADR only when *all three* hold: (1) a real alternative was on the table, (2) the choice constrains future work, (3) the cost is non-trivial and not inferable from the decision itself. If only one or two hold, the decision lives in `2_design.md` prose, as a row in an existing ADR's table, or as a task-ID citation on the parent ADR's status line. Every ADR must include at least one bullet labeled `Cost:`. See [CONVENTIONS.md §4](../CONVENTIONS.md).

## Consequences

- The log stays readable as a list of *load-bearing* decisions; readers can scan the titles and grasp the architectural shape of the feature.
- Cross-review enforces the rule; rejected ADRs are downgraded to design-doc prose or removed.
- Cost: the rule is judgmental. Reasonable agents disagree on whether a given decision crosses the bar, and reviews occasionally re-litigate the rule itself rather than the substance. There is no automated check today; lint cannot verify "is this decision load-bearing" because the question is semantic. The rollup-fold mechanic ([CONVENTIONS.md §4a](../CONVENTIONS.md)) is the maintenance escape hatch when a cluster of accepted ADRs turns out to instantiate the same underlying choice.