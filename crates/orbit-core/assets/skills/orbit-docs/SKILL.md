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
related_artifacts: ["<task-id>", "<adr-id>", "<learning-id>"]
---
```

`type` and `summary` are required. `summary` must be a non-empty single line. `related_artifacts` uses ID-prefix dispatch for task, learning, friction, and ADR IDs.

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

Doc = explanatory context. It is PR-reviewed Markdown, retrieved through `orbit.docs.*`, and has no supersede flow. Link to load-bearing learnings with `related_artifacts: [L-NNNN]` when useful.

## Routing Notes

- ADRs are owned by `orbit-adr` and live at `.orbit/adrs/{accepted,proposed,superseded}/<adr-id>/`. Orbit-docs does not walk `.orbit/`, but `orbit search <query> --kind all` and `orbit search <query> --kind adr` federate ADR metadata alongside doc hits. Use `--all` to include superseded ADRs for archaeology.
- For the boundary rationale, run `orbit tool run orbit.adr.list --input '{"feature":"orbit-docs"}'` and inspect the accepted ADR that covers the sibling-index search overlay.
- Learnings are owned by `orbit-learning`; cross-reference them from docs with `related_artifacts`.
- `orbit-design` is retired. Use `orbit-docs` for docs retrieval and `orbit-adr` when creating, accepting, or superseding ADRs.

## Tool Invocation

CLI and MCP forms are equivalent:

| Verb | CLI | MCP |
| --- | --- | --- |
| List | `orbit docs list --json` | `orbit.docs.list` |
| Show | `orbit docs show <path> --json` | `orbit.docs.show` |
| Search | `orbit search --kind doc <query> --json --limit 20` (or `--kind all` / `--kind adr`) | `orbit.search` |
| Add root | `orbit docs add <path>` | `orbit.docs.add` |
| Reindex | `orbit docs reindex` | `orbit.docs.reindex` |
| Migrate | `orbit docs migrate --dry-run` | `orbit.docs.migrate` |

`orbit search <query> --kind all` federates doc and ADR hits in one call. Use `--kind doc` for doc-only matches, `--kind adr` for ADR-only matches, and `--kind adr --all` (or `--status adr:accepted,adr:superseded`) to include superseded ADRs for archaeology. See `orbit-search` for the full flag matrix (`--tag`, `orbit search path <path>`, `--status kind:value`).

`reindex` is a v1 no-op because the indexer walks on demand. `migrate` backfills locked frontmatter for `docs/design/<feature>/*.md` and `docs/design-patterns/*.md`; it never touches `.orbit/`.

## Workflow

1. Use `orbit search <query> --kind all --json` first when looking for context across docs and ADRs (or `--kind doc` / `--kind adr` to narrow).
2. Use `orbit docs show <path> --json` for the full Markdown body.
3. Use `orbit docs list --json --type <type>` or `--tag <tag>` when browsing.
4. Use `orbit docs add <path>` only for existing non-`.orbit/` roots that should be searched going forward.
5. Use `orbit docs migrate --dry-run` before writing frontmatter backfills, then rerun without `--dry-run` when the diff is expected.
