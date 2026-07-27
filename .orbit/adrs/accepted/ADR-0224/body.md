## Context
CLI providers can exit successfully after persisting authoritative task, review, and git artifacts while emitting prose or provider wrapper JSON that lacks an Orbit response envelope. Treating every missing or malformed envelope as fatal strands completed work; treating every response as advisory would break activities whose downstream templates consume structured fields.
## Decision
CLI agent-loop activities treat response envelopes as best-effort by default. Exit status and timeout determine transport success, valid envelopes still project result fields, and parse failures become bounded redacted diagnostics. An activity sets `require_response_envelope: true` only when downstream workflow steps consume its structured response.
## Consequences
- Artifact-backed implementation and review runs no longer fail solely because final agent prose is malformed or missing an envelope.
- Structured-output activities remain fail-closed through an explicit per-activity contract.
- Cost: Activity authors must classify the handoff correctly, and response-consuming activities need a regression that pins strict mode.