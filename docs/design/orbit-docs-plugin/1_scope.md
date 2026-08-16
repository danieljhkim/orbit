---
type: design
summary: "Scope: extract docs + search into a plugin-style feature crate"
tags: [orbit-docs-plugin]
last_validated: 2026-08-09
---

# Scope: extract docs + search into a plugin-style feature crate

Status: draft (uncommitted) — scoping for the docs+search pluginization pilot.
Extraction precedent: ORB-10016 ([Extract the CLI-facing command layer into orbit-cmd](../orbit-core/4_decisions.md#extract-the-cli-facing-command-layer-into-orbit-cmd)).

Learning-specific comparisons below are retained only as historical scoping
context. [ORB-10736] / [Remove the native project-learning subsystem](../project-learnings/4_decisions.md#remove-the-native-project-learning-subsystem) remove the native learning resource.

## Goal

Move the docs + search feature out of orbit-core into a library plus embedded
command-layer pair:

- **`orbit-docs`** (feature crate): corpus domain — `DocRecord`, docs-root walking, frontmatter,
  doc/ADR lexical search-source builders, doc/ADR embedding-source builders, doc/ADR index +
  search orchestration. Depends on `orbit-common`, `orbit-search`, `orbit-store` (Adr types) only.
- **`orbit-docs-cli`** (thin clap `Command` enum + `run(ctx)`): embedded by orbit-cli under
  `orbit docs` / `orbit search` / `orbit semantic`, [Workspace_path-addressable MCP host tools with surface-scoped containment](../mcp-session-context/4_decisions.md#workspacepath-addressable-mcp-host-tools-with-surface-scoped-containment) style. Depends only on `orbit-docs`.

orbit-core becomes a *consumer* of `orbit-docs` (same directional shape as today's
`orbit-core -> orbit-search` edge). orbit-core's tool hosts keep working by calling into the
feature crate through thin `OrbitRuntime` delegate methods.

## Why this shape (and not the Explore-report "Path C" as written)

A crate holding the command modules cannot depend on orbit-core **and** be called by orbit-core's
tool hosts — that's a cycle. The resolution is inversion of the data flow, not of traits:
extracted functions take explicit context (docs roots, search configs, `&VectorStore`, record
lists) instead of `&OrbitRuntime`. No orphan-rule exposure: orbit-core implements nothing foreign;
it just builds the context and calls plain functions. This sidesteps the objection that kept
docs/search in core during ORB-10016.

## What moves / what stays

| Piece | Today | Disposition |
|---|---|---|
| `orbit-core/src/command/docs/` (~2.3k LOC, 13 modules) | corpus walking, frontmatter, list/show/add, doc+ADR source builders, index params, migrate | **moves** to `orbit-docs` |
| `orbit-core/src/command/search/` doc/ADR branches | hybrid scoring branches of `global_search` | **moves** to `orbit-docs` |
| `global_search` orchestration + task/learning branches | merges 4 domains into `GlobalSearchResponse` | **stays** in orbit-core (cross-domain merger is runtime-level; it consumes `orbit-docs` for the doc/ADR branches) |
| `command/semantic.rs` (139 LOC) | thin dispatch to orbit-search commands | **stays** as thin delegates (or trims further) |
| Tool hosts (`docs_tools`, `search_tools`, `semantic_tools`, `task_tools::related_docs_for_context`, ~220 LOC) | expose feature as agent tools | **stay** in orbit-core; repointed to call `orbit-docs` via delegates |
| `builder.rs` VectorStore + EmbedWorker init; `store_delegates.rs` task enqueue/delete cascade | task-mutation indexing plumbing | **stays** — task-domain, out of scope for this pilot |
| `orbit-search` (7.8k LOC leaf) | scoring, vector store, companion RPC, install/uninstall | **unchanged** |
| CLI dispatch (`orbit-cli/src/command/{docs,search,semantic}.rs`) | thick Execute impls over runtime methods | **replaced** by embedded `orbit-docs-cli::Command`; orbit-cli builds ctx from runtime |

## Phases (each lands as its own PR into agent-main; orbit is PR+CI-gated)

1. **Characterize.** Golden-output tests over `orbit docs list/show/index`, `orbit search`,
   `orbit semantic stats/index` (JSON snapshots). These gate every later phase.
2. **Extract `orbit-docs`.** `git mv` the docs modules + doc/ADR search branches; convert
   `&OrbitRuntime` params to an explicit `DocsContext` (roots, configs, `&VectorStore`).
   orbit-core keeps delegate methods so orbit-cli/tool hosts compile unchanged. Add the
   dependency-direction guard edge (`orbit-core -> orbit-docs`, never reverse).
3. **Plug at orbit-cli.** Create `orbit-docs-cli` (Command enum + run), embed under the existing
   subcommand names; delete the thick Execute impls. Fix the `orbit docs index` →
   `semantic_index(IndexKind::Docs)` cross-call while touching it — give docs its own index verb.
4. **Trim + document.** Repoint tool hosts through the delegates onto `orbit-docs`; trim
   orbit-core re-exports freed by the move; ARCHITECTURE.md, website crate docs, stability
   markers (`orbit-docs`: internal), and the ADR recording the boundary.

## Size

~5.5k LOC moved (mostly mechanical, `git mv`-preserving), ~500–700 LOC new glue
(context struct, CLI crate, guard, tests). Comparable to ORB-10016 in mechanics, smaller in
API-surface churn. 4 PRs.

## Risks

- **Golden-output drift** — hybrid ranking is fiddly; phase 1 snapshots are the mitigation.
- **Config leakage** — `DocsContext` must stay data-only; if config *layering* logic creeps into
  `orbit-docs`, the boundary failed. Layering resolution stays in orbit-core.
- **`global_search` split line** — doc/ADR branch extraction must not change merged ranking;
  covered by snapshots.
- **EmbedWorker semantics** — async, lossy-by-design (batches, drops on queue-full, debug-level
  failures). Out of scope; do not "fix" in passing.

## Open questions

1. Naming: `orbit-docs` vs `orbit-corpus`? (ADR + learning embedding sources also live in the
   corpus — "docs" undersells it slightly.)
2. Do the `orbit semantic` verbs move under the plugin CLI or stay core-side? (Install/uninstall
   are companion lifecycle — arguably orbit-search's CLI, not docs'.)
3. Standalone binary or embed-only? The separately installed
   `orbit-search-companion` is justified by its heavyweight optional inference
   dependencies; docs commands have no comparable isolation need. Standalone
   also requires config resolution without `OrbitRuntime`, so prefer embed-only
   for the pilot.
4. Does `LearningEmbeddingSource` projection (built inline in semantic.rs from
   `runtime.list_learnings()`) stay core-side? Suggest yes — learnings are runtime domain.
