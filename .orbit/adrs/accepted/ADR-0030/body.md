## Context
Audit needs useful payloads for reproducibility, but raw provider keys or sensitive environment-derived values would make the trail unsafe by default.

## Decision
Redact sensitive environment values, HTTP authorization patterns, API-key fields, bearer tokens, and selected argv token shapes before durable blob or error-message persistence.

## Consequences
- Audit readers can treat normal stored blobs as already redacted.
- Cost: redaction changes payload hashes and may remove exact bytes useful for reproducing a provider interaction.
