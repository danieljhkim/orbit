## Context
Groundhog wants stable checkpoint memory that can be serialized incrementally. Rewriting prior chronicle bytes would make cache-friendly prefix reuse impossible if the runtime ever leans on those helpers.

## Decision
Keep an append-only chronicle serializer contract where earlier serializations are byte-exact prefixes of later ones.

## Consequences
- The runtime has a reusable primitive for stable checkpoint-memory serialization.
- Chronicle history can grow without mutating prior serialized bytes.
- Cost: current runtime persistence is still split across `Chronicle` and `groundhog/state.json`, so the serializer's benefits are only partially realized today.
