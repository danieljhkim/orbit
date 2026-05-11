## Context
`git log --follow` chases renames through history but at non-trivial per-hop cost. Hunk coordinates have to be re-mapped after every rename hop. At commit volumes typical of this repo, follow mode compounds into minutes of extra walker time. This decision described the now-removed attribution walker.

## Decision
Map hunks to leaves by line-range overlap against the symbol's span *at the commit's tree*. Do not chase renames. A symbol moved across files gets attribution from post-move commits only.

## Consequences
- Walker cost is predictable and linear in commits, not in rename hops.
- Pure deletions credit the insertion-point symbol — approximation on purpose.
- Cost: a symbol moved across files loses attribution from pre-move commits. Agents investigating long-lived code history may see gaps. See [2_design.md §6.3] for the full caveat.

---
