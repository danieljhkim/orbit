## Context
Common ADR systems (RFCs, IETF, Rust) include `withdrawn` or `rejected` states for proposals that were considered and abandoned. The question is whether ADR-artifact needs them.

## Decision
Three states only: `proposed`, `accepted`, `superseded`. A proposed ADR that won't ship is **deleted** by its owner (file moved under `.orbit/adrs/deleted/` for archaeology). An accepted ADR the team backs out of is **superseded** by a new ADR that explains the reversal.

## Consequences

- Tool surface stays small; lifecycle transitions are unambiguous.
- The "we considered X and rejected it" record lives in the *winning* ADR's Context section, not as a separate withdrawn record. Forces the reasoning to live next to the chosen path, which is more useful for future readers.
- Cost: lossy. A speculative proposal an owner deletes is gone from the corpus; if someone later wants to revisit the same idea, the old proposal isn't preserved as a discoverable record. The deleted-folder archaeology is a partial mitigation, not a search-indexed one.

---
