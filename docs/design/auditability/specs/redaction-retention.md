# Spec: Redaction and Retention Boundaries

Audit storage must preserve enough detail to reconstruct agent behavior while redacting known secret shapes before durable persistence.

## Why This Exists

Auditability and secrecy pull in opposite directions. Orbit needs faithful records, but it must not make provider keys, bearer tokens, or sensitive environment values durable by accident.

## Redaction Invariants

- BlobStore redacts bytes before computing the stored hash.
- Command audit error messages are scrubbed for sensitive live environment values before insertion.
- Pipeline outputs persisted by runtime helpers are scrubbed for sensitive live environment values.
- HTTP-shaped payload redaction covers authorization headers, x-api-key headers, JSON API-key fields, and bearer tokens.
- Shared pattern redaction covers high-confidence provider token shapes embedded in prose; exact whole-token artifact fields are rejected instead of persisted.
- CLI argv redaction uses HTTP defaults plus bare `sk-...` token scrubbing when argv-shaped data is being persisted.
- Orbit artifact write tools use the action-keyed field policy in [artifact-redaction.md](./artifact-redaction.md) before YAML/markdown/JSON persistence.
- Default tracing output redacts string field values, `Debug`-formatted field values, and unstructured `message` fields before writing stderr or `~/.orbit/state/logs/orbit.jsonl`.
- Readers should not need to apply the standard redactor again for normal stored blobs.

## Retention Boundaries

Command audit rows:

- Live in the configured SQLite audit database.
- Are queryable, exportable, and prunable through `orbit audit`.
- Should remain compact and should not embed transcript bodies.

Activity/job and loop traces:

- Live under `.orbit/state/audit/` for workspace-local run reconstruction.
- Use JSONL for append-friendly event streams.
- Use content-addressed blobs for payload bodies.
- May be manually retained or deleted with workspace state until a first-class retention policy exists.

Invocation metrics:

- Live in SQLite as usage records.
- May be recomputed or summarized into scoreboards.
- Are not a transcript retention mechanism.

Global process tracing:

- Lives under `~/.orbit/state/logs/orbit.jsonl`.
- Is append-only and unrotated in v1.
- Is an operational log stream, not the canonical workflow envelope.
- Carries policy-denial path/resource strings and friction summaries after [T20260427-0023], so default tracing redaction is part of its durability boundary.

## Failure Modes

- If a redactor misses an unknown secret shape, the audit layer may persist that value. Reviewers should treat new provider payload shapes as redaction-sensitive changes.
- If redaction changes payload bytes, the stored hash identifies the redacted payload, not the raw provider payload.
- If JSONL writes fail, the run may continue with in-memory audit snapshots only; durable reconstruction is incomplete.
- If pruning deletes command audit rows, file-backed run traces and blobs are not automatically pruned unless a future retention task adds that coupling.

## Migration Path

Future retention work should add:

- a single operator command that reports command-row, JSONL, blob, job-run, and invocation retention by workspace
- optional hash manifests for file-backed audit bundles
- audit records for export and prune operations
- a documented legacy handling policy for pre-`.orbit/state/audit/` run traces

## Agent Signature

Last revised by codex / gpt-5.5 for [T20260427-0023].
