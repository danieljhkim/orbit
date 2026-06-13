## Context
The original draft hardcoded "10ms stat budget at 5000 files; cache window 500ms" inside the query layer. The budget doesn't scale, and the policy mixes product decisions into the library.

## Decision
`Graph::open(root, policy: SyncPolicy)` where `SyncPolicy` is `Manual | OnRead | Windowed { window: Duration }`. CLI default: `Manual`. MCP server default: `Windowed { window: 500ms }`.

## Consequences
- Tests use `Manual` for determinism; long-lived processes use `Windowed`; one-shot scripts can use `OnRead` for paranoia.
- The library no longer carries an implicit perf contract that breaks silently at scale.
- Cost: **callers must choose.** No "just works" default beyond per-entry-point conventions. The conventions are documented but the choice is exposed.