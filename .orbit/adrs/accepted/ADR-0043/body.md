## Context
`summary.json` used token/invocation scoreboard tool-call totals, which can be empty for providers that do not emit invocation traces, while command audit records every tool-run attempt.

## Decision
Count `command: tool` rows with `subcommand: "run"` or `"run-mcp"` and `tool_name` present as scoreboard all/failed tool-run attempts; keep token totals sourced from invocation/token scoreboards.

## Consequences
- Failed and denied tool runs become visible in compact summaries even for trace-sparse providers.
- Cost: the legacy max overlay is conservative and may undercount the true union until both streams share an invocation id.
