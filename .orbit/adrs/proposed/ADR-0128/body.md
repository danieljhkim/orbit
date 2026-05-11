## Context
The first-draft scope was reduced to its minimal core — substrate (derivation, bipartite bridge, storage) plus a single read surface. Two candidate read surfaces were on the table: (a) the symbol-biography surface that renders an artifact's history on load, or (b) the authoring-assist hook that runs at task creation. Only one fits the "minimal substrate proof" framing.

## Decision
The symbol-biography surface (`orbit.lineage.biography`, the §6 renderer, and the `feature` closure substrate) is the minimal Phase 1 read surface. Authoring assist is **deferred** — it is one downstream consumer of the biography substrate and ships in a follow-up phase. Stale-task detection, ADR auto-supersession, and the auto-fire-on-PR "review assist" hook are also deferred (the last is explicitly removed; on-demand `orbit.lineage.reversal` covers reviewer-initiated cases when those closures eventually land).

## Consequences
- The minimal Phase 1 ships only one consumer (the biography read surface) of the substrate. That consumer is *the* read surface that operationalizes the "oral history for agents" vision; the kill criterion in ADR-001 binds to biography-surface usage.
- Authoring assist, which would be the first *write-back* consumer, lands later. The minimal first draft proves the substrate carries weight before any write-back consumer commits to it.
- Cost: the minimal Phase 1 has no write-back proof. If the biography surface drives usage but no downstream write-back ever materializes, the substrate is read-only forever. That outcome is acceptable — read-only oral history is still load-bearing — but the design must be honest that "consumers will follow once the substrate ships" is a bet, not a guarantee.

---
