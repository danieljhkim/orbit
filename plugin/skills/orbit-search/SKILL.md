---
name: orbit-search
description: Use when searching tasks, docs, learnings, or ADRs through the unified `orbit search` query surface; also covers the `orbit semantic` embedding-companion lifecycle, and the human-authored docs corpus — searching, listing, showing, registering, reindexing, or migrating docs (locked frontmatter schema, recommended docs layout, learning-vs-doc boundary, ADR routing).
---

# Orbit Search

Use `orbit search` to find project context by topic, literal phrase, or related task ID. The query surface is `orbit search`; the lifecycle surface is `orbit semantic install|uninstall|stats|index`. `orbit graph` remains the tool for code-structure questions (callers, refs, implementors, symbol selectors) — search is corpus retrieval, graph is structural traversal.

## Query Surface

| Tool | MCP | CLI |
|------|-----|-----|
| `orbit.search` | `orbit_search({...})` | `orbit tool run orbit.search --input '{...}'` |

Include `model` in JSON inputs for provenance (your agent family). See the `orbit` skill for the full surface-mapping rule.

```bash
# Lexical global search across tasks, docs, learnings, and ADRs
orbit search "slow inference after model swap" --limit 5

# Cross-artifact label and path filters (--tag is AND when repeated)
orbit search "scheduler" --tag perf --kind all
orbit search path crates/orbit-search/src/lib.rs --kind all

# Hybrid lexical + cosine over indexed fields
orbit search "agent loop deadlock" --hybrid --kind task --limit 5

# Cosine neighbors of a known task (requires the semantic companion)
orbit search similar "<task-id>" --limit 5   # MCP: {"semantic":"<task-id>","limit":5}
```

`orbit search path <path>` applicability lookup: task results use selector containment over `context_files`; learning/ADR results use glob-containment over stored path scopes; docs are content-indexed and don't match by applicability path. `--status` values use `kind:value` tokens (e.g. `--status task:open,doc:active,adr:proposed`) — bare tokens are rejected since statuses collide across corpora.

**Index coverage:** lexical covers tasks, docs, learnings, ADRs. Vector search covers task fields plus docs/learnings/ADRs after `orbit semantic index --kind docs|learnings|adrs`. Missing vectors under `--hybrid` fall back to lexical with a note rather than failing.

**When to use:** (1) pre-create dedup check — `--hybrid --kind task` before adding a task; (2) pre-execute related lookup — `semantic: "<task-id>"` after `orbit.task.show` to surface prior decisions the author may not have linked; (3) ad-hoc — "where did we decide X" starts with `orbit search "X" --kind all`.

**Stop rule:** if one well-formed query returns useful hits, stop and inspect — don't chain rewrites chasing higher scores. Don't use search for "find every symbol matching X" — that's `orbit.graph.search`.

## Semantic Companion Lifecycle

`orbit semantic` manages the local embedding companion — it is not the query namespace.

| Purpose | Form |
|---------|------|
| Install / remove | `orbit semantic install [--model M] [--force]` / `orbit semantic uninstall [--model M] [--all]` |
| Status | `orbit semantic stats` |
| Rebuild embeddings | `orbit semantic index --kind tasks\|docs\|learnings\|adrs\|all [--model M] [--force]` |

Supports macOS arm64 and Linux x86_64/aarch64 with glibc ≥ 2.38; no x86_64-apple-darwin asset. Don't run `install` without operator consent; if a semantic query fails because the companion is missing, fall back to lexical `orbit search --kind <kind> <query>` and continue unless the user explicitly opted in.

Result shape: `mode`, `kind`, `notes`, `results[]` — each with `kind`, `source`, and some of `id`/`path`/`title`/`summary`/`status`/`best_field`/`snippet`/`score`/`matched_by`. Treat scores as relative ordering, not confidence; read snippets/matched fields before judging relevance.

## Docs Corpus

Orbit docs are PR-reviewed Markdown under configured `[docs].roots` (default `docs/`) — designs, reusable patterns, domain notes, glossaries, runbooks. Registration-light: Orbit walks configured roots on demand and indexes files with valid frontmatter (tolerant fallback for legacy design/pattern docs).

Frontmatter (`type` and `summary` required; `summary` non-empty single line):

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

Recommended (not enforced) layout: `docs/design/<feature>/`, `docs/design-patterns/`, `docs/context/`, `docs/glossary.md`, `docs/runbooks/`.

**Learning vs Doc:** a learning is a load-bearing rule with a known failure mode — managed through `orbit.learning.*` (see `orbit-knowledge`), scope-glob push-injected, updatable/supersedable. A doc is explanatory context — PR-reviewed, retrieved via `orbit.search --kind doc`, no supersede flow. Link a doc to a load-bearing learning with `related_artifacts: [L-NNNN]`.

**Routing:** ADRs are owned by `orbit-knowledge` and live at `.orbit/adrs/{accepted,proposed,superseded}/<adr-id>/` — docs does not walk `.orbit/`, but `orbit search --kind all|adr` federates ADR metadata alongside doc hits (`--all` includes superseded ADRs for archaeology). `orbit-design` is retired in favor of this tolerant surface.

**Admin/CLI-only workflows** (agents use `orbit.search` for retrieval; these are human/admin):

| Verb | Form |
| --- | --- |
| List | `orbit docs list --json [--type <type>] [--tag <tag>]` |
| Show | `orbit docs show <path> --json` |
| Add root | `orbit docs add <path>` (existing non-`.orbit/` roots only) |
| Index | `orbit docs index --json` (after substantial edits/moves; idempotent via content hashes) |
| Migrate | `orbit docs migrate --dry-run` then without, to backfill locked frontmatter for legacy `docs/design/<feature>/*.md` and `docs/design-patterns/*.md` — never touches `.orbit/` |

## Common Mistakes

| Mistake | Why it fails | Correct form |
|---------|---------------|--------------|
| Calling lifecycle commands to search | `orbit semantic` manages the companion | Use `orbit search` / `orbit.search` |
| Aborting when the companion isn't installed | Embeddings are optional infra | Fall back to lexical unless the user opted in |
| Using semantic search for exact identifiers | Lexical is cheaper/predictable for names, paths, error strings | Plain `orbit search` or `orbit.graph.search` |
| `--semantic <id>` on a brand-new task | May not have embeddings yet | `--hybrid --kind task` on title/description |

## Cross-References

- `orbit-graph` — code-structure queries.
- `orbit-task` — pre-create dedup check, pre-execute related-task lookup.
- `orbit-knowledge` — learnings and ADRs this skill retrieves but does not author.
