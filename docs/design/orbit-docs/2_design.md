---
title: "Orbit Docs — Design"
owner: claude
last_updated: 2026-05-21
status: Draft
feature: orbit-docs
doc_role: design
type: design
summary: "Orbit Docs — frontmatter schema, walker, doc embeddings index, hybrid search, and the `.orbit/` exclusion invariant."
tags: [orbit-docs]
paths: ["crates/orbit-core/src/command/docs.rs", "crates/orbit-cli/src/command/docs.rs"]
related_features: [orbit-docs]
related_artifacts: [ORB-00163, ORB-00206, ADR-0169, ADR-0170, ADR-0171, ADR-0180]
---

# Orbit Docs — Design

This document specifies what [ORB-00163] and [ORB-00206] ship: the locked frontmatter schema, the strict-then-tolerant parser, the walker (including the `.orbit/` exclusion invariant), the six-verb CLI / MCP surface, doc-corpus embeddings, hybrid doc search, and the migration verb that backfills legacy docs. It also names the remaining limitations the follow-ups ([ORB-00164] through [ORB-00169]) address.

The design lives in two files: [crates/orbit-core/src/command/docs.rs](../../../crates/orbit-core/src/command/docs.rs) (~1290 lines, parser + walker + verb implementations + tests) and [crates/orbit-cli/src/command/docs.rs](../../../crates/orbit-cli/src/command/docs.rs) (~250 lines, clap argument shapes + table rendering). The MCP twin lives in [crates/orbit-core/src/runtime/orbit_tool_host/docs_tools.rs](../../../crates/orbit-core/src/runtime/orbit_tool_host/docs_tools.rs).

---

## 1. Frontmatter Schema

The schema is locked at six fields. Two are required; four are optional. See [ADR-0169] for why the schema is closed.

```yaml
---
type: design | pattern | context | glossary | runbook    # required
summary: One-line hook for agent retrieval                # required
tags: [hook, learning, audit]                             # optional
paths: ["crates/orbit-cli/**"]                            # optional
related_features: [hook-rewrite]                          # optional
related_artifacts: [ORB-00160, ADR-0168, L-0003]     # optional
---
```

### 1.1 `type` (required)

Strict enum. The parser rejects any value outside `design | pattern | context | glossary | runbook`. The five categories were chosen to be coarse enough that every Markdown doc in the repo falls into exactly one without ambiguity, but granular enough that filtering by type is useful at retrieval time.

The tolerant indexer infers `type` from directory layout when the field is absent: see §3.2.

### 1.2 `summary` (required)

Non-empty single line. The parser trims trailing whitespace, then errors if either (a) the value is empty after trim, or (b) the value spans more than one line. Multi-line scalars are not supported by the strict parser.

`summary` is the primary retrieval hook. `orbit search --kind doc` scores against `summary` substring matches with the highest single-field weight (see §5.2). The expectation is that authors write `summary` as an *agent-readable retrieval cue*, not a doc title: it should answer "if an agent searched for the concept this doc covers, what phrase would they search?"

### 1.3 `tags` (optional)

Free-form string list. Used for `--tag` filtering in `orbit docs list` and as an exact-match scoring signal in `orbit search --kind doc`. There is no controlled vocabulary; teams are free to converge on their own tagging conventions.

The tolerant indexer populates `tags` with the feature slug for design docs (e.g. `tags: [activity-job]` for `docs/design/activity-job/...`).

### 1.4 `paths` (optional)

Glob string list. Names file paths the doc applies to. This is the join key for hook-time scoping ([ORB-00167]): when an agent is about to Edit / Read / Write a file, the hook can surface docs whose `paths` glob matches.

Not used by `orbit search --kind doc` ranking in v1; the field exists for the injection wiring.

### 1.5 `related_features` (optional)

Free-form string list. The join key for task-time scoping ([ORB-00166]): when an agent runs `task show <id> --with-context`, the renderer can surface docs whose `related_features` overlaps with the task's `related_features`.

### 1.6 `related_artifacts` (optional)

String list with ID-prefix dispatch (see [ADR-0171]):

| Prefix shape | Resolves to |
|--------------|-------------|
| `ORB-NNNNN` | Orbit task |
| `L-NNNN` | Project learning |
| `F<YYYY>-<MM>-<NNN>` | Friction report |
| `ADR-NNNN` | Architecture decision record |

The parser hard-errors on unknown prefixes. This is intentional: silent acceptance of `XYZ-1` would let typos rot in the corpus.

---

## 2. Strict Parser

Strict-mode parsing is the contract the `migrate` verb backfills toward, and the contract `orbit docs add`-ed roots are expected to honor. Strict mode is invoked by `parse_doc_frontmatter_strict` and is the inner workhorse of the tolerant `parse_doc_tolerant`.

### 2.1 Frontmatter delimiter

The first line of the file must be exactly `---` (optionally with `\r`). Anything else means "no frontmatter, fall back to tolerant inference." A frontmatter block must be terminated by a second `---` line. Unterminated frontmatter is a hard error in strict mode and falls through to tolerant inference otherwise.

### 2.2 YAML deserialization

The block between the two `---` delimiters is fed to `serde_yaml::from_str::<RawDocFrontmatter>`. `RawDocFrontmatter` mirrors the six fields with all-`Option`-or-`Vec` types; required-field enforcement happens *after* deserialization so we can produce field-specific error messages.

### 2.3 Required-field enforcement

- `type`: missing → `OrbitError::InvalidInput("frontmatter in <path> is missing required field type")`.
- `summary`: missing → similar error. After trimming, empty → another error. Multi-line → another error.

The errors carry the file path so `orbit docs list` failures are actionable.

### 2.4 Output

A `DocFrontmatter` struct with the six fields, ready to serialize as JSON for the CLI / MCP shape.

---

## 3. Tolerant Indexer

Strict mode is the canonical contract. Tolerant mode is what makes the corpus queryable on day one without a flag-day migration. It is the path most reads go through ([crates/orbit-core/src/command/docs.rs:368](../../../crates/orbit-core/src/command/docs.rs)).

### 3.1 Algorithm

```
parse_doc_tolerant(repo_relative, absolute_path, raw):
    if strict-parse succeeds → return parsed
    body := split_frontmatter(raw)?.body  OR  raw
    return ParsedDoc {
        frontmatter: infer_frontmatter(repo_relative, body),
        body,
    }
```

The body is extracted by skipping the frontmatter block if one exists, even malformed; otherwise the whole file is the body. This means a doc with a broken frontmatter still surfaces its content; only the strict-parse error is suppressed.

### 3.2 `infer_type_and_tags`

| Repo-relative path shape | Inferred `type` | Inferred `tags` |
|--------------------------|-----------------|-----------------|
| `docs/design/<feature>/<filename>.md` (depth ≥ 4) | `design` | `[<feature>]` |
| `docs/design-patterns/<filename>.md` | `pattern` | `[]` |
| Path contains `runbooks` component | `runbook` | `[]` |
| Filename stem or path contains `glossary` | `glossary` | `[]` |
| Anything else | `context` | `[]` |

### 3.3 `infer_summary`

Walk body lines until the first non-empty, non-frontmatter, non-HTML-comment line. Strip `#` heading markers, surrounding backticks, and angle-bracket noise. If everything was stripped, fall back to a titleized filename stem (`error_translation` → `Error Translation`); ultimate fallback is `"Untitled document"`.

---

## 4. Walker

`walk_docs_roots(repo_root, roots)` is the entry point for `orbit docs list`. It returns a sorted, deduplicated `Vec<DocRecord>` (path + frontmatter).

### 4.1 Root expansion

Each entry in `[docs].roots` is treated as either a literal path or a wildcard pattern. Wildcards (`*` only; full glob is out of scope for v1) are expanded by recursive directory listing. Non-existent literal roots are silently ignored (no error), so a misconfigured root doesn't break the walker.

### 4.2 The `.orbit/` exclusion (load-bearing)

Two layers of defense, enforced by [ADR-0170]:

1. **Per-root precheck.** Before descending into a configured root, `path_is_or_contains_dot_orbit` rejects it if any path component is `.orbit`. This catches `docs/.orbit/...` and any misconfigured root pointing at or under `.orbit/`.
2. **Per-file recheck.** Inside `maybe_push_doc`, the same check runs again on every Markdown file. Catches the case where a configured root is *above* `.orbit/` (e.g. someone sets `roots = ["."]` to index everything) and the recursion descended past the per-root check.

A unit test (`walker_skips_dot_orbit_even_when_root_points_above_it`) pins the contract: a tempdir with `.orbit/adrs/ADR-0001/body.md` under a configured root must produce zero records starting with `.orbit/`.

### 4.3 Other skips

- `.git`, `node_modules`, `target` — hard-listed in `should_skip_dir`.
- `.gitignore`-matched paths — `is_git_ignored` shells out to `git check-ignore -q` per file. This is slow at scale; [ORB-00164] tracks the fix (batched stdin or the `ignore` crate).

### 4.4 Deterministic output

Records are sorted by repo-relative path before deduplication. `orbit docs list --json` is reproducible across runs in an unchanged corpus.

---

## 5. Search

`orbit search --kind doc <query> [--limit N]` ranks docs by substring and exact-match scoring across `summary`, `tags`, and `type`.

### 5.1 Scoring

| Field | Hit condition | Score |
|-------|---------------|-------|
| `summary` | Lowercased substring match | `80 + len(query)` |
| `type` | Lowercased substring match against type name | `30` |
| `tags` | Exact case-insensitive match against any tag | `120` |
| `tags` | Substring match against any tag | `60` |

A record with zero hits is dropped. Ties are broken by path order (deterministic).

### 5.2 Why these weights

Exact tag match is the strongest signal because authors writing tags are committing to a categorization. Summary substring is next because authors write `summary` knowing it's the retrieval hook. Type contributes weakly so that `orbit search --kind doc runbook` surfaces runbooks above pattern docs that incidentally mention "runbook" in their summary.

### 5.3 Limit and default

Default limit is 20. `--limit N` (CLI) or `limit` field (MCP) caps results. v1 does not paginate; if a corpus grows past 20 hits for a typical query, the limit can be raised.

### 5.4 What's missing

- No body-text scoring. The whole point of frontmatter is that the body is unstructured; ranking against arbitrary Markdown adds noise without semantic ranking.
- No semantic embeddings. Deferred to [ORB-00168].
- No `paths` or `related_features` scoring. These fields exist for *injection-time* scoping (hook + task.show), not for direct search.

---

## 6. The Six Verbs

### 6.1 `orbit docs list`

Wraps `walk_docs_roots` with optional `--type` and `--tag` filters. Both filters are case-insensitive and additive (a doc must match both when both are set). Empty `roots` configuration is not an error: returns `[]`.

JSON shape: `[{ "path", "type", "summary", "tags", "paths", "related_features", "related_artifacts" }, ...]`.

### 6.2 `orbit docs show <path>`

Reads the named repo-relative path, parses tolerantly, returns the parsed frontmatter plus the body. Errors when (a) the path doesn't exist, (b) the path is under `.orbit/`, or (c) the path is outside configured `[docs].roots`. The third check uses `canonicalize` to follow symlinks.

JSON shape: `{ "path", "frontmatter", "body" }`.

### 6.3 `orbit search --kind doc <query>`

See §5. Returns the ranked list with `score` and `matched_by` (list of which fields hit). Without `--hybrid`, the output is lexical-only. With `--hybrid`, Orbit embeds the query, retrieves `source_kind = "doc"` rows, min-max normalizes lexical and cosine scores, and blends them with `[docs.search].semantic_weight` (default `0.5`, clamped to `[0.0, 1.0]`). If the companion is missing or no doc embeddings exist, the command warns and falls back to lexical results.

### 6.4 `orbit docs add <path>`

Idempotently appends a normalized path to `[docs].roots`. Refuses non-existent paths and `.orbit/` paths. Writes back to `.orbit/config.toml`, preserving the rest of the file by round-tripping through `toml::Value`. Idempotency is determined by normalized path comparison (trailing `/` and case ignored on the candidate-vs-existing check).

### 6.5 `orbit docs index [--json] [--force] [--model <alias>]`

Walks configured docs roots, reads each Markdown body through the tolerant parser, maps each doc to `{path, title, tags, body}`, and calls the orbit-search vector store with `source_kind = "doc"`. `--force` bypasses the existing `content_hash` skip, while the default path is idempotent. After indexing, the command deletes stale doc rows whose `source_id` no longer appears in the live docs corpus.

### 6.6 `orbit docs migrate [--dry-run]`

Scans `docs/design/<feature>/*.md` (relative depth 2) and `docs/design-patterns/*.md` (relative depth 1) for files that don't pass strict parsing. For each, runs tolerant inference and either:

- If the file has no frontmatter: prepends a fresh `---` block with `type`, `summary`, and `tags`.
- If the file has frontmatter that's missing locked fields: `upsert_yaml_scalar` line-edits in `type` and `summary`, and appends `tags` if absent.

Idempotent: a second run reports `No docs need migration.` `--dry-run` prints planned diffs without writing. Never touches `.orbit/`.

The line-based YAML editing is fragile against multi-line / quoted values — [ORB-00164] tracks the round-trip-through-`serde_yaml` fix.

---

## 7. MCP Surface

Five tools, registered in `safe_mcp_tool_names` ([crates/orbit-cli/src/command/mcp/mod.rs](../../../crates/orbit-cli/src/command/mcp/mod.rs)):

```
orbit.docs.list
orbit.docs.show
orbit.docs.add
orbit.docs.index
orbit.docs.migrate
```

The per-domain doc-search MCP tool was retired by [ORB-00202]; content-similarity queries now route through the unified `orbit.search` tool with `kind: "doc"`. Each remaining tool's `execute` forwards to `OrbitBuiltinAction::Docs*` and routes through the same `OrbitRuntime` methods the CLI uses. CLI and MCP shapes are identical. Audit events for MCP invocations land in the same SQLite store as CLI events, tagged `subcommand: "run-mcp"`.

---

## 8. Config

`.orbit/config.toml` recognizes:

```toml
[docs]
roots = ["docs/"]                     # default when section absent
# roots = ["docs/", "apps/*/docs/"]   # monorepo example

[docs.search]
semantic_weight = 0.5                 # default; clamped to 0.0..1.0
```

When the section is absent or the file is empty, the default root is `["docs/"]` and the default hybrid semantic weight is `0.5`. `parse_docs_roots_from_config_toml` and `parse_docs_search_config_from_config_toml` are the inner parsers; `OrbitRuntime::docs_roots` and `OrbitRuntime::docs_search_config` are the call sites that resolve the configured path.

---

## 9. Concerns & Honest Limitations

- **Lexical search is still frontmatter-only.** Plain `orbit search --kind doc` scores summary + tags + type only. Body-level concept recall requires `orbit docs index` plus `orbit search --kind doc --hybrid`.
- **`migration_diff` is not a real diff.** It prints the first 12 lines of `before` and first 16 lines of `after` glued together with `@@` markers. Misleading label. [ORB-00164] tracks the fix.
- **`update_existing_frontmatter` hand-edits YAML by line.** Works for simple `key: scalar` legacy headers. Will misbehave on multi-line block scalars (`description: |`) and quoted values containing colons. [ORB-00164] tracks the round-trip-through-`serde_yaml` fix.
- **`is_git_ignored` shells out per file.** Acceptable at ~100 docs. Will degrade at thousands. [ORB-00164] tracks the batched-stdin or `ignore`-crate fix.
- **No hook-time or task-time injection yet.** The retrieval primitive ships in v1; injection is [ORB-00166] and [ORB-00167].
- **Doc semantic freshness is explicit.** Task writes enqueue background embeddings, but docs require `orbit docs index` until a future watcher/background indexer exists.
- **ADRs are not in the corpus.** [ORB-00169] is the design question.

---

## Task References

- [ORB-00163] — Introduce `orbit docs` indexed knowledge base and `orbit-docs` skill
- [ORB-00164] — Harden orbit-docs internals: real diff, robust YAML edit, gitignore caching
- [ORB-00166] — Wire `orbit docs` retrieval into `task.show --with-context` and `task.start`
- [ORB-00167] — Extend PreToolUse hook to surface relevant docs alongside learnings
- [ORB-00168] — Add semantic embeddings index for orbit-docs corpus (v2)
- [ORB-00169] — Design: fold `.orbit/adrs/` into the orbit-docs corpus (v2)

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
