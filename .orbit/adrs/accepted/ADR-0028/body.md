## Context
Activity/job execution needs run, step, retry, fan-out, loop, and activity structure. Provider loops need HTTP, tool-call, payload, and session detail.

## Decision
Use `V2AuditEnvelope` for activity/job structure and `LoopAuditEvent` for provider/tool detail, connected through run ids and parent event ids.

## Consequences
- Workflow replay can traverse a run tree without loading every provider payload.
- Cost: reviewers need tooling or documentation to move between related files.
