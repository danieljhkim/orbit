## Context
Knowledge needs globally allocated identifiers without making a hub checkout or a stale replica a second author.
## Decision
The hub allocates global IDs, the declared owner authors current knowledge, and Git replicas are opt-in reads marked as replicas; the hub never proxies to a spoke owner.
## Consequences
- A non-owner agent files work for the owner rather than writing through a new route.
- Cost: finalize failures consume valid-but-unused IDs and current spoke-owned knowledge is unavailable off-owner.