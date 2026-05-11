## Context
A profile that grants `modify: ["./build/**"]` without granting `read: ["./build/**"]` is technically valid but produces a confusing operational story: a tool may be allowed to write a file it cannot read, breaking the standard read-modify-write pattern.

## Decision
`PolicyDef::validate` rejects any profile whose positive `modify` rule is not covered by a positive `read` rule in the same profile. "Covered" is checked structurally (`rule_covers_path_rule`): exact match, `**`, or a `<prefix>/**` rule that prefixes the modify rule.

## Consequences
- Modify rules require corresponding read coverage, so read-modify-write audit stories stay consistent.
- Cost: profile authors who *only* want to allow append-style writes cannot express that without granting a read rule. There is no "write-only" profile shape today.
