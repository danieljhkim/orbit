## Context

After [ADR-0174] and [ADR-0175] consolidated `orbit search` as the unified query surface, the per-domain `search` subcommands (`orbit task search`, `orbit docs search`, `orbit learning search`) became redundant for content-similarity queries. Worse, `orbit learning search` was bundling three unrelated operations under one verb: substring search (content), path-glob applicability lookup (structural), and tag filter (structural). Two of those are filters dressed up as search.

At the same time, agents pre-edit need a single command that answers *given this file path, what tasks / learnings / ADRs apply here?* — the context-pack assembly query. In the final CLI shape after ADR-0179, that is the `orbit search path <path>` form; MCP keeps a `path` parameter. The same logic applies to `--tag`: one cross-kind label bridge.

The phase-1 search engine already supported `--kind {task,doc,learning,adr,all}`. Phase 2 finishes the consolidation: removes the redundant verbs, re-homes their filters under the unified search surface, and fixes one observed semantics bug in `learning list --path`.

## Decision

Five threads decided together because they share a single mental model — *search = content-similarity, list = structural filter*:

1. **Deletion verdicts.** Hard-remove `orbit task search`, `orbit docs search`, `orbit learning search` (CLI + MCP). No deprecation shims — phase 1 set the precedent that no external consumers depend on these surfaces. Replacement: `orbit search <query> --kind <X>` for content-similarity queries; `orbit <kind> list --filter` for structural filters.

2. **Structural-vs-content split.** `search` carries free-text or neighbor queries against indexed content. `list` carries structural filters (status, tags, paths, owners). `orbit learning search --path` and `--tag` cases re-home onto `orbit learning list --path` / `--tag`. The substring case re-homes onto `orbit search <query> --kind learning`.

3. **Universal status wideners replace per-kind flags.** Introduce `--all` (kind-aware widener) and `--status <kind:value,...>` on `orbit search`. Per-kind defaults: task = `proposed,backlog,in-progress,review` (+ `done,rejected,archived` on `--all`); learning = `active` (+ `superseded` on `--all`); adr = `proposed,accepted` (+ `superseded` on `--all`); doc = no-op. The old `orbit docs search --include-superseded` mental model is replaced by `orbit search --kind adr --all`. One vocabulary, three kinds covered.

   *Implementation note:* `AdrStatus` does not currently carry a `Deprecated` variant, so `--all` adds `Superseded` only. If a deprecated state is added later, the widener will pick it up without a flag change.

4. **Path lookup and `--tag` as cross-kind filters.** Both compose with `--kind` and with each other. The CLI spells path lookup as `orbit search path <path>`; the MCP tool keeps a `path` parameter. Per-kind semantics:
   - **`--tag`**: AND semantics for repeated values; case-insensitive. Applies to task, doc, learning, and the union (`all`). For `--kind adr` the filter returns empty and `--help` documents the deferral; the underlying constraint is that ADRs have no free-form `tags` field today (`related_features` is structural).
   - **Path lookup**: applies to task and learning. For task, selector-mapping against `context_files` (`file:` exact, `dir:` containment in either direction, `symbol:` matches on file component). For learning, glob-containment against `scope.paths`. ADR and doc return empty; help text states the deferral.
   - Cross-kind ADR tag and path matching is deferred to phase 3 (ORB-00203) which adds the necessary frontmatter fields.

5. **`orbit learning list --path` semantics flip.** From exact-match (`scope.paths.iter().any(|p| p == path)`) to glob-containment (compile each rule as a glob regex, match the normalized query path). This aligns `learning list --path` with what the deleted `learning search --path` did, which is what the pre-edit context-pack use case needs. This is the only observable behavior change in phase 2; everything else is surface consolidation.

## Consequences

- One mental model for search: `orbit search` queries indexed content; `orbit <kind> list` filters structural metadata. The boundary is enforced by the flag layout, not by convention.
- The agent context-pack query collapses to a single command: `orbit search path <file> --kind all`. Previously this was three separate calls plus client-side merging.
- `--all` and `--status kind:value` give every kind the same widening vocabulary; reviewers reading a script with `--all` know what it does without checking per-kind flag tables.
- Phase-3 (ORB-00203) gets a clean specification for ADR `paths` and `tags`: `orbit search path X --kind adr` and `orbit search <query> --tag X --kind adr` already exist as no-op branches; phase 3 fills them in without changing the public surface.
- `learning list --path` now matches the intuition that *a learning with `scope.paths: [src/auth/**]` applies to `src/auth/login.rs`*. The behavior flip is called out in the CHANGELOG; the previous exact-match semantics were never documented as load-bearing for any agent flow.
- Audit middleware sheds the `Search` arms on Task/Docs/Learning subcommands. Audit event names `orbit.task.search`, `orbit.docs.search`, `orbit.learning.search` are orphaned by the hard break, matching the no-shim policy.
- Cost: the `learning list --path` semantics flip is a real behavior change. Any script or skill calling `orbit learning list --path src/auth/**` expecting exact-match behavior will now also see paths inside that glob. Mitigated by: (a) `learning list` returned no matches before the flip when called with a concrete file path under a glob scope, so almost all real-world calls were broken anyway; (b) the new behavior is what the deleted `learning search --path` already did, so the migration target for ex-`learning search --path` users is unchanged.
- Cost: ADR carries tag and path no-ops until phase 3 lights them up. Documented in `--help` so users do not construct queries that silently return empty.
- Cost: `AdrStatus` lacks a `Deprecated` variant; `--all` widening on ADRs is asymmetric with the task widener (which gets multiple terminal states). A separate task can extend `AdrStatus` if a deprecated state ever becomes load-bearing.