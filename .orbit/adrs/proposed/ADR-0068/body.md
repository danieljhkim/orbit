## Context
This ADR was written when `task_ids` attribution was still a graph feature. That feature was removed in ADR-029 / [T20260506-11], so the attribution-preservation motivation is historical. The remaining symbol-operation taxonomy may still inform future graph write tools, but it no longer has an attribution consumer.

## Decision
Formalize the contract that hook validates against via [specs/graph-operations.md](./specs/graph-operations.md). Eight-op taxonomy (`create`, `delete`, `rename`, `move`, `change_signature`, `change_body`, `split`, `merge`), each atomic per entry — compound refactors emit multiple entries rather than a single "relocate" primitive. Address symbols by a graph-level **stable ID** (`stable_id: node:<nanoid-21>`) persisted on every node; rejected pre/post address pairs because disambiguation under simultaneous axis changes and N-ary ops (`split`/`merge`) requires a subject handle equivalent to a stable ID under a different name. Operations are **advisory-authoritative**: accepted when consistent with the commit's tree diff, ignored with a warning otherwise — the tree is always ground truth.

## Consequences
- The taxonomy is deliberately small — eight ops cover every refactor shape Orbit intends to own without requiring compound primitives.
- Any future producer needs a new consumer and migration story; attribution preservation is no longer enough rationale.
- Cost: `stable_id` would still be a new field on `BaseNodeFields` if revived — schema bump on first rebuild after producer lands, and a one-time reallocation of object hashes for every existing node. Also: status stays `Proposed` until the producer ships; flip to `Accepted` + the producer's task ID at that time.

---
