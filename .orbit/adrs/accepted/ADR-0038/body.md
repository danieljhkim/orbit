## Context
`orbit log tail` established terminal semantics, but the dashboard needed the same source/code/message vocabulary without copying formatter logic into browser JavaScript.

## Decision
Extract log formatter/filter/path logic into a shared `orbit-cli` module and expose dashboard `/api/log` snapshot plus `/api/log/stream` SSE endpoints that render escaped `message_html` server-side.

## Consequences
- CLI, dashboard backend, and dashboard UI share one log vocabulary and escaping boundary.
- Cost: stream rotation/truncation handling is best-effort, and the visual panel ships separately under UI ownership.
