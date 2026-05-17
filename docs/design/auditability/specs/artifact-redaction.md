# Spec: Artifact Write Redaction

ADR and learning artifact tools redact user-supplied free text before writing
YAML, markdown, or JSONL artifact files. This is a write-boundary guarantee:
once persisted, the redacted form is canonical and read paths do not need to
repair it.

## Tool Fields

- `orbit.adr.add` and `orbit.adr.update`: redact `title` and `body`.
- `orbit.adr.supersede`: no free-text artifact fields; the success response
  still reports `redactions_applied: false`.
- `orbit.learning.add` and `orbit.learning.update`: redact `summary`, `body`,
  `scope.tags`, and `evidence[].ref`.
- `orbit.learning.add` and `orbit.learning.update`: apply only home-directory
  normalization to `scope.paths` so glob syntax is preserved.
- `orbit.learning.comment.add`: redact `body`.
- `orbit.learning.supersede`: no free-text artifact fields; the success
  response still reports `redactions_applied: false`.

Every redacted write response includes `redactions_applied: true` when any
persisted field differs from the caller's input. Unchanged writes include
`redactions_applied: false`.

## Refuse Versus Mask

All masking goes through `orbit_common::utility::redaction::redact_all` followed
by `redact_home_dir`, except `scope.paths`, which uses `redact_home_dir` only.
The default pattern redactor masks embedded credential-shaped tokens in prose.

A field is refused with `OrbitError::SensitiveInput` when the entire trimmed
value is one high-confidence credential token and it was not already scrubbed
as a live sensitive environment value. The explicit token formats are:

- `^sk-[A-Za-z0-9_-]{20,}$`
- `^ghp_[A-Za-z0-9]{36}$`
- `^xox[baprs]-[A-Za-z0-9-]{10,}$`

The same token embedded in larger prose is masked in place, not rejected. A
live `GITHUB_TOKEN` value is also masked as `[REDACTED_ENV]` instead of being
refused, because the environment redactor can prove the source of the value.

## Idempotence

Redaction markers are stable. Running `redact_all(redact_all(x))` must equal
`redact_all(x)` for artifact fixtures, and learning tag normalization preserves
`[REDACTED_*]` markers rather than lowercasing them.

## Audit Events

Each field that changes emits a command-audit event with
`target_type: artifact_redaction`, the artifact ID, the field name, and the
redaction kinds applied: `env`, `pattern`, and/or `home_dir`. Audit payloads
must not contain either original values or redacted values.

## Non-Goals

Structural IDs, lifecycle statuses, owner/model identity, related task IDs,
related feature names, legacy IDs, validation warnings, priorities, evidence
kinds, supersession IDs, and comment parent IDs are not redacted by this
artifact-write boundary.

## Task References

- [ORB-00137]
