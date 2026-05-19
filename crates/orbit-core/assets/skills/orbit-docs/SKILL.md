---
name: orbit-docs
description: Use when searching, listing, showing, registering, reindexing, or migrating the human-authored docs corpus via `orbit.docs.*`. Covers the locked frontmatter schema, recommended docs layout, learning-vs-doc boundaries, and ADR routing.
---

# Orbit Docs

## Purpose

Use this skill when the user asks to search the docs, inspect the docs corpus, show a doc, index this doc, register a docs root, or migrate legacy docs frontmatter.

Orbit docs are PR-reviewed Markdown under configured `[docs].roots` (default `docs/`). They carry explanatory context: designs, reusable patterns, domain notes, glossaries, and runbooks. The surface is intentionally registration-light: Orbit walks configured roots on demand and indexes files with valid frontmatter, with tolerant fallback for legacy design and pattern docs.

## Frontmatter Schema

```yaml
---
type: design | pattern | context | glossary | runbook
summary: One-line hook for agent retrieval
tags: [hook, learning, audit]
paths: ["crates/orbit-cli/**"]
related_features: [hook-rewrite]
related_artifacts: [ORB-00160, ADR-0168, L20260514-3]
---
```

`type` and `summary` are required. `summary` must be a non-empty single line. `related_artifacts` uses ID-prefix dispatch: `ORB-NNNNN` for tasks, `LYYYYMMDD-N` for learnings, `FYYYY-MM-NNN` for friction reports, and `ADR-NNNN` for ADRs.

## Recommended Layout

This is a recommendation, not an enforcement rule:

- `docs/design/<feature>/` for feature and architecture narrative.
- `docs/design-patterns/` for reusable codebase patterns.
- `docs/context/` for domain, product, or operational background.
- `docs/glossary.md` or `docs/glossary/` for shared vocabulary.
- `docs/runbooks/` for operational procedures.

Orbit-docs indexes any configured Markdown root with valid frontmatter; it does not require the four-numbered design-doc layout.

## Learning vs Doc

Learning = a load-bearing rule with a known failure mode. It is managed through `orbit.learning.*`, has scope-glob push injection, and can be updated, superseded, or pruned.

Doc = explanatory context. It is PR-reviewed Markdown, retrieved through `orbit.docs.*`, and has no supersede flow. Link to load-bearing learnings with `related_artifacts: [L<YYYYMMDD>-N]` when useful.

## Routing Notes

- ADRs are owned by `orbit-adr` and live at `.orbit/adrs/{accepted,proposed,superseded}/ADR-NNNN/`. Orbit-docs does not index `.orbit/` in v1.
- Learnings are owned by `orbit-learning`; cross-reference them from docs with `related_artifacts`.
- Use `orbit-design` only when the user specifically wants the current design-doc convention tooling. Orbit-design retirement is separate from orbit-docs.

## Tool Invocation

CLI and MCP forms are equivalent:

| Verb | CLI | MCP |
| --- | --- | --- |
| List | `orbit docs list --json` | `orbit.docs.list` |
| Show | `orbit docs show <path> --json` | `orbit.docs.show` |
| Search | `orbit docs search <query> --json --limit 20` | `orbit.docs.search` |
| Add root | `orbit docs add <path>` | `orbit.docs.add` |
| Reindex | `orbit docs reindex` | `orbit.docs.reindex` |
| Migrate | `orbit docs migrate --dry-run` | `orbit.docs.migrate` |

`reindex` is a v1 no-op because the indexer walks on demand. `migrate` backfills locked frontmatter for `docs/design/<feature>/*.md` and `docs/design-patterns/*.md`; it never touches `.orbit/`.

## Workflow

1. Use `orbit docs search <query> --json` first when looking for context.
2. Use `orbit docs show <path> --json` for the full Markdown body.
3. Use `orbit docs list --json --type <type>` or `--tag <tag>` when browsing.
4. Use `orbit docs add <path>` only for existing non-`.orbit/` roots that should be searched going forward.
5. Use `orbit docs migrate --dry-run` before writing frontmatter backfills, then rerun without `--dry-run` when the diff is expected.
