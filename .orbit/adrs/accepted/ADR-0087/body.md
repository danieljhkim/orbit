## Context
`orbit.graph.search` default ranking previously asked the service for `usize::MAX` hits so ranking could choose the best `limit` results from the full match set. On large graphs this let a small user-facing limit retain an effectively unbounded candidate list before ranking.

## Decision
Replace the unbounded request with a named headroom multiplier and hard cap. Default ranking collects more candidates than the requested `limit`, ranks that bounded pool, and returns the top `limit`; filtered and source-regex searches keep their explicit limit behavior.

## Consequences
- Broad default searches retain a bounded candidate set before ranking.
- Queries whose strongest matches fit inside the capped candidate pool keep the same ranking and output order.
- The tool description now states the cap so callers know very broad default searches can rank only the retained candidate pool.
- Cost: if the best-ranked match appears after the cap in service traversal order, it is no longer considered until a narrower query, type/kind filter, or prefix is supplied.

---
