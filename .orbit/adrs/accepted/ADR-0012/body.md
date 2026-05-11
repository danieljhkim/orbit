## Context
`task_auto_pipeline` needed to skip its success guard for empty backlog runs, but its seeded `bundle_count > 0` guard rendered to an unsupported comparison and failed before the step could be skipped. Orbit could either extend the shared evaluator with numeric ordering or express the guard in the existing grammar.

## Decision
Keep the shared condition grammar to `==` and `!=`, with `&&` and `||` composition, and express skip-on-empty guards with equality-compatible forms such as `!= 0` and `!= []`. The `ship-auto` guard uses `{{ steps.validate_bundles.output.bundle_count }} != 0`, so zero bundles skip the guard and populated fan-out still checks child gate success.

## Consequences
- The evaluator stays string-based and shared between `StepCondition::Expr`, v2 `when:`, and loop `break_when:` without adding numeric coercion rules.
- Seeded jobs can still model empty collections and counts, but authored guards must avoid ordering operators unless a future task intentionally extends the grammar.
- Cost: authors cannot write natural numeric comparisons in guards today; they must encode supported equality checks or add a deliberate grammar extension with tests and docs.
