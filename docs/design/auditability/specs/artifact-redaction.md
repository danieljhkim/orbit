# Spec: Artifact Write Redaction

Orbit artifact tools persist repo-backed YAML, markdown, and JSON. Their write boundary sanitizes selected input fields after `OrbitBuiltinAction` is known and before typed params are built. The sanitizer is action-keyed rather than field-blind, so structural IDs, statuses, tags, and artifact blobs keep their native validation rules.

## Field Policy

| Tool | Free text (`redact_all` + `redact_home_dir`) | Path-only (`redact_home_dir`) | Skip |
|------|----------------------------------------------|-------------------------------|------|
| `orbit.adr.add` / `orbit.adr.update` | `title`, `body` | - | status, owner, related ids/features/tasks, legacy ids |
| `orbit.adr.supersede` | - | - | `old_id`, `new_id` |
| `orbit.learning.add` / `orbit.learning.update` | `summary`, `body`, `scope.tags[]`, `evidence[].ref` | `scope.paths[]` | status, ids, priority, votes, model fields |
| `orbit.learning.supersede` | - | - | `old_id`, `new_id` |
| `orbit.learning.comment.add` | `body` | - | `learning_id`, `model` |
| `orbit.task.add` | `title`, `description`, `plan`, `acceptance_criteria[]`, `comment` | `context_files[]`, `context`, `external_refs[].url` | workspace, ids, enums, dependency/relation targets, crew, tags |
| `orbit.task.update` | `title`, `description`, `plan`, `execution_summary`, `acceptance_criteria[]`, `comment` | `context_files[]`, `context` | provenance/status/identity fields, tags, raw artifacts |
| `orbit.task.reject` | `note`, `comment` | - | `id` |
| `orbit.task.review_thread.add` | `body` | `path` | `id`, `line`, `model` |
| `orbit.task.review_thread.reply` | `body` | - | `id`, `thread_id`, `model` |
| `orbit.friction.add` | `body` | - | `model`, `during_task`, tags |
| `orbit.friction.update` | `body` | - | `id`, status, tags |

Task and friction tags are taxonomy fields and pass through verbatim. Learning `scope.tags[]` are matching metadata and are treated as free text.

## Refuse vs Mask

Free-text fields reject a value that is exactly one high-confidence credential token:

- `^sk-[A-Za-z0-9_-]{20,}$`
- `^ghp_[A-Za-z0-9]{36}$`
- `^xox[baprs]-[A-Za-z0-9-]{10,}$`

The rejection is a typed `OrbitError::SensitiveInput` and never includes the token value. The same token shapes embedded in larger prose are masked by the shared redaction module. Path-only fields only normalize HOME-prefixed strings; token-shaped globs and character classes are preserved.

## Response and Audit Contract

Covered mutating tools add `redactions_applied: bool` to object responses. It is `true` only when at least one persisted field changed from the caller's input. Re-running a write with already-redacted text is idempotent: no field changes means `redactions_applied: false` and no redaction audit event.

When a field changes, Orbit emits one command-audit row per field. The payload contains only:

- artifact type and id
- field path
- actor
- tool name
- redaction kinds: `env`, `pattern`, `home_dir`

Original and redacted values are not recorded. Tests should inspect these rows through the same backing surface as `orbit audit list --json` per L-0009.

## Non-Goals

- Display-time/read-time redaction.
- Raw task artifact blob redaction through `orbit.task.artifact.put` or task `artifacts` / `upsert_artifacts`.
- General-purpose schema validation in the sanitizer; existing typed parsers keep reporting shape errors.
- Auto-publish, commit, or remote cleanup behavior after a secret has already entered history.
