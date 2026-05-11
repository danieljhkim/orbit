## Context
A durable JSONL feed made tracing output persistent, but call-site helpers only protected emitters that remembered to use them.

## Decision
Install redacting `FormatFields` implementations on stderr and JSONL tracing formatters so string fields, `Debug` values, and messages are scrubbed before output.

## Consequences
- New structured tracing emitters inherit default redaction before terminal or disk output.
- Cost: span attribute redaction, binary payload redaction, and user-configurable policies remain follow-up concerns.
