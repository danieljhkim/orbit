## Context

The decay check needs a per-doc freshness timestamp to compare against `git log` of referenced source files. Two anchors were on the table: (a) parse `Last updated:` from frontmatter, requiring authors to bump it manually; (b) use `git log -1 --format=%cs -- <doc>.md` directly. Option (b) eliminates the manual-discipline failure mode; option (a) carries an explicit author assertion.

## Decision

Use the `Last updated:` field. The author updates it manually whenever the doc body changes substantively; cosmetic edits (typo fixes, link reflows, whitespace) intentionally do *not* bump it. The check parses the field and trusts it.

## Consequences

- The freshness signal carries an explicit semantic: "the author has read this doc end-to-end and asserts it still describes the system." `git log` of the doc cannot carry that semantic.
- Cosmetic-only PRs do not falsely reset the staleness clock for a six-month-stale doc.
- The discipline is enforceable by review (and eventually a pre-commit hook, [3_vision.md §1.4](./3_vision.md)) but not by the decay check itself.
- Cost: an author who forgets to bump the date ships a doc that looks fresh until the next reviewer notices. This is the dominant failure mode of the system today; it is accepted as the price of the explicit-assertion semantic.