## Context

`task.yaml` currently stores metadata, long prose, acceptance criteria, comments, history, and review threads together. This makes simple tasks easy to inspect, but it turns every content edit or append into a YAML rewrite and makes Markdown-hostile fields harder for humans and agents to author.

## Decision

Keep `task.yaml` as a small structured envelope and move prose into Markdown sidecars: `description.md`, `acceptance.md`, `plan.md`, and `execution-summary.md`. Public APIs may expose logical string/list fields, but storage treats the files as source of truth.

## Consequences


- Prose gets native Markdown editing, diffs, and rendering.
- YAML becomes smaller, easier to validate, and easier to merge.
- CLI/tool reads should treat sidecars as first-class documents rather than maintaining embedded-YAML compatibility.
- Cost: one task now spans more files. Simple scripts that read only `task.yaml` must switch to the bundle API.

## Provenance

Migrated verbatim from the local heading `task-artifacts/ADR-002` in `docs/design/task-artifacts/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-05 · Phase 3 v2 bundle primitives (`c14fa640`); Phase 4 document update hardening (`06847332`)