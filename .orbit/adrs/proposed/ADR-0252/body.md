## Context

The v1 runtime-host phase-out (knowledgebase/polaris/design/orbit-cleanup/phaseoutv1.md, Stage 3) deletes the v1 executor stack once planning duel is ported to v2 (ORB-10393). `ExternalExecutor` implements the External Executor Protocol v1 — a documented public extension point (docs/design/executors/specs/external-executor-protocol.md, ADR-0196, assets/executors/external.example.yaml) — and shares the `direct_agent` subprocess transport slated for deletion. The phase-out design flagged an open question: if external executors remain a supported surface, the transport must be rehomed rather than deleted.

## Decision

Daniel decided on 2026-07-25: the External Executor Protocol is not a supported surface and is retired. `ExternalExecutor` and the shared `direct_agent` transport are deleted with the rest of the v1 executor stack in Stage 3 (ORB-10395); nothing is rehomed. The protocol spec doc and example asset are marked retired/removed in the same change, and ADR-0196 is superseded by this record.

## Consequences

- Stage 3 becomes a pure deletion with no transport-rehoming work; the v1 executor stack (~1,000+ LOC) drops out in one gated task.
- Any out-of-tree executor built against the protocol stops working; there are no known consumers, and the removal is noted in release notes.
- Cost: re-introducing an external-executor extension point later means designing a new protocol against the v2 pipeline from scratch rather than reviving this one.