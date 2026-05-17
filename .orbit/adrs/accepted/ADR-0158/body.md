## Context

Per-feature design folders need a contract that makes cross-feature reading cheap. Free-form folders (every author picks their own structure) and single-doc folders (one big README) were both on the table. The first failure mode of free-form folders had already surfaced in early Orbit work: readers had to learn each folder's structure before they could find the decision history.

## Decision

Every feature folder contains exactly four numbered markdown docs with fixed roles: `1_overview.md` (what and why), `2_design.md` (current implementation), `3_vision.md` (forward-looking), `4_decisions.md` (ADR log). A reader who learns the contract once can navigate any feature folder without re-orienting. Required-section lists per file ([CONVENTIONS.md §3](../CONVENTIONS.md)) lock in section order so that "open `2_design.md` and read the mechanism numbered §3" is a stable instruction.

## Consequences

- Cross-feature reading is cheap: the same file name means the same thing everywhere.
- Authors who would have written one doc are forced to write four, even when the feature is small.
- The required-section contract is checkable; today by review, eventually by lint ([3_vision.md §1.2](./3_vision.md)).
- Cost: tiny features (a single mechanism with no open questions and no real decisions) end up with shallow `3_vision.md` and `4_decisions.md` files. The fix is not "drop the file" but "the feature was probably too small for its own folder" — promote the work into an existing folder instead. This pressure is sometimes ignored, leaving a few thin folders.