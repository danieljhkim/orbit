---
name: orbit-search
description: Search tasks, docs, ADRs, and frictions through the unified `orbit search` surface. Also covers the `orbit semantic` embedding companion and the human-authored docs corpus.
---

# Orbit Search

`orbit search` finds project context by topic, literal phrase, or related task ID. It is corpus retrieval, not structural traversal — for callers, refs, implementors, or symbol selectors, read files with `fs.read` or use `rg`.

The query surface is `orbit.search` (MCP `orbit_search({...})`, CLI `orbit tool run orbit.search --input '{...}'`). Include `model` for provenance. The *lifecycle* surface — `orbit semantic install|uninstall|stats|index` — manages the embedding companion and is not a way to query anything.

```bash
orbit search "slow inference after model swap" --limit 5          # lexical, all corpora
orbit search "scheduler" --tag perf --kind all                    # --tag is AND when repeated
orbit search path src/lib.rs --kind all                           # applicability lookup
orbit search "agent loop deadlock" --hybrid --kind task --limit 5 # lexical + cosine
orbit search similar "<task-id>" --limit 5                        # MCP: {"semantic":"<task-id>"}
```

**Applicability (`search path`)** resolves differently per corpus: tasks match by selector containment over `context_files`; ADRs match by glob containment over their decision scopes; docs are content-indexed and never match by path.

**`--status` takes `kind:value` tokens** (`--status task:open,doc:active`). Bare tokens are rejected because statuses collide across corpora.

**Index coverage:** lexical covers tasks, docs, ADRs, and frictions. Vector search covers task fields and docs once `orbit semantic index --kind <kind>` has run. Missing vectors under `--hybrid` fall back to lexical with a note rather than failing — and if the companion isn't installed at all, fall back to lexical and continue. Never run `orbit semantic install` without operator consent.

**Where it earns its keep:** a `--hybrid --kind task` dedup check before creating a task; `semantic: "<task-id>"` after `orbit.task.show` to surface prior decisions the author never linked; and "where did we decide X" as `--kind all`. For a brand-new task, use `--hybrid` on title and description — it may not have embeddings yet. For exact identifiers, paths, and error strings, plain lexical is cheaper and more predictable than semantic.

**Stop rule:** if one well-formed query returns useful hits, stop and inspect. Don't chain rewrites chasing a higher score.

Results carry `mode`, `kind`, `notes`, and `results[]` with some of `id`/`path`/`title`/`summary`/`status`/`best_field`/`snippet`/`score`/`matched_by`. Scores are relative ordering, not confidence — read the snippet and matched field before judging relevance.

## Corpora

**Docs** are PR-reviewed Markdown under configured `[docs].roots` (default `docs/`) — designs, patterns, domain notes, glossaries, runbooks. Orbit walks the roots on demand and indexes anything with valid frontmatter. Authoring a doc, registering a root, or migrating legacy files: [references/docs-corpus.md](references/docs-corpus.md).

**ADRs** are complete entries in the repository's designated decision docs. They are indexed as ordinary docs; use `--kind doc` and search by ID, title, or body text. The retired `--kind adr` is rejected.
