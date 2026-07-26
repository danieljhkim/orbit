## Context

Current task artifacts are `path + UTF-8 content`. That is enough for planning duel Markdown or JSON, but it excludes screenshots, binary logs, trace bundles, and generated media. It also lacks checksums and media-type metadata.

## Decision

Store artifacts under `artifacts/files/` and track them with `artifacts/manifest.yaml`. Each manifest entry records logical path, blob path, media type, checksum, size, and attribution. Public `TaskArtifact` values carry raw bytes plus media type so writers and readers do not reintroduce UTF-8-only assumptions above the manifest layer.

## Consequences


- Tasks can carry screenshots, binary traces, and structured generated outputs without abusing text fields.
- Artifact integrity can be checked independently of the task envelope.
- CLI display can choose text rendering, summaries, or file paths based on media type.
- Cost: artifact write/read code becomes more complex, and storage now needs size limits, redaction checks, and checksum validation.

## Provenance

Migrated verbatim from the local heading `task-artifacts/ADR-006` in `docs/design/task-artifacts/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-05 · Phase 6 public artifact DTO surgery (working tree)