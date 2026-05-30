---
name: orbit-learning
description: Use this when creating, searching, updating, superseding, pruning, or auditing Orbit project learnings. Agent tools cover `orbit.learning.add/show/update/supersede/comment.add` plus `orbit.search`; list/sync/prune/comment.list/comment.delete/upvote are CLI-only operator workflows. Covers scope-OR matching (path globs / tags), evidence shape, the `update` vs `supersede` boundary, the YAML-source-of-truth + SQLite-index model, and why never to edit `.orbit/learnings/` files directly.
---

# Orbit Learning

## Purpose

Curate durable, scope-injected guidance that survives across sessions and agents. Learnings are the third Orbit primitive alongside tasks (work) and ADRs (decisions): a learning is a piece of *push-injected context* — when an agent is about to touch a file, directory, or workflow that matches a learning's `scope`, the relevant learnings get injected into the agent's prompt automatically. This skill is the **pull/curate** side of that loop: how to author new learnings, find existing ones, replace stale guidance, and archive what no longer applies. The convention itself lives in [docs/design/project-learnings/](../../../docs/design/project-learnings/).

Use this skill when the user reports a recurring failure mode, when a code review surfaces a non-obvious gotcha worth preserving, when an incident root-cause turns into a guardrail, or when a workflow insight ("we always X before Y in this crate") is worth pushing to future agents. Do **not** use it for one-off task notes — those belong on the task itself.

Learning artifact files are written into the current worktree's
`.orbit/learnings/L-NNNN/` subtree, while IDs are allocated globally through
the shared allocator. Stage new learning files from your task worktree
alongside the code/doc change that produced the guidance; sibling worktrees
will see remote stubs until those body files are locally readable. The
canonical learning ID format is `L-NNNN`; legacy `L<YYYYMMDD>-N` IDs were
migrated by ORB-00200 and should only appear in `legacy_ids`.

## Tool Invocation

Both surfaces accept the same JSON. Use the CLI examples when shell access is available; use the MCP names when the Orbit plugin exposes them.

| Tool / workflow | MCP | CLI |
|------|-----|-----|
| `orbit.learning.add` | `orbit_learning_add({...})` | `orbit learning add --summary "..." --path "crates/orbit-core/**/*.rs" --tag rust --body-file note.md` |
| Search learnings | `orbit_search({...})` | `orbit search --kind learning <text>` (lexical free-text match; add `--hybrid` after `orbit semantic index --kind learnings` for lexical + semantic ranking) |
| `orbit.learning.show` | `orbit_learning_show({...})` | `orbit learning show --id L-0001` |
| `orbit.learning.comment.add` | `orbit_learning_comment_add({...})` | `orbit learning comment add --learning-id L-0001 --body "Narrow note" --model codex` |
| Learning comment list | CLI-only (operator) | `orbit learning comment list --learning-id L-0001` |
| `orbit.learning.comment.delete` | CLI-only (admin) | `orbit learning comment delete --id C20260514-1` |
| `orbit.learning.update` | `orbit_learning_update({...})` | `orbit learning update --id L-0001 --priority 200` |
| `orbit.learning.supersede` | `orbit_learning_supersede({...})` | `orbit learning supersede --id L-0001 --with L-0007` |
| `orbit.learning.prune` | CLI-only (admin) | `orbit learning prune --stale-only` |
| List/audit learnings | CLI-only | `orbit learning list --status active --tag rust` (also `--path <glob-or-file>`) |
| Sync YAML envelope index | CLI-only | `orbit learning sync` |

Mapping rule: `orbit.learning.<verb>` ↔ `orbit_learning_<verb>`. Always include `model` in JSON inputs when the tool accepts it; pass your agent family (`codex`, `claude`, `gemini`, or `grok`). Prefer `--body-file` for `add` and body-changing `update` calls so multi-line markdown is not mangled by shell quoting.

Run `orbit tool list | grep orbit.learning` if you suspect the local active tool surface has drifted; use `orbit tool list --all` when auditing CLI-only inactive tools.

## Workflow

1. **Search before adding.** Before creating a new learning, check whether one already covers the same scope:
   - `orbit learning list --path <path-you-care-about>` for path-anchored guidance (glob-containment).
   - `orbit learning list --tag <tag>` for cross-cutting topics.
   - `orbit search --kind learning <substring>` for lexical content match against `summary` and body (case-insensitive).
   - `orbit search --kind learning --hybrid <query>` when the semantic companion is installed and learning embeddings have been indexed with `orbit semantic index --kind learnings`; this blends lexical ranking with semantic matches over learning summary/body/tags and falls back to lexical results when vectors are unavailable.
   If a near-match exists, prefer `update` (refine the existing record) or `supersede` (replace it with a new ID) over creating a duplicate.

2. **Add with tight scope.** A learning is only useful when its `scope` triggers the right injections — not too broad (noise) and not too narrow (never fires). `scope: { paths?, tags? }` matches as **paths OR tags** (a record fires when *any* path glob OR *any* tag hits). Include `evidence: [{ kind: "task"|"commit"|"external", ref: "..." }]` whenever the learning came from a real incident, PR, or task — future readers (and prune logic) lean on it. Use `priority` (0–255) sparingly; it is the secondary search ranking key, not a "this is important" badge.

3. **Close the loop with a source citation when the learning has a code anchor.** If the learning captures a code-level convention — a defensive pattern, gotcha, non-obvious workaround, or specific default that survives in a single (or small set of) source location(s) — drop a one-line citation comment at each such location so the next reader sees the rationale before they reach for the change:

   ```rust
   // L-NNNN: <one-line rationale>
   ```

   Use the literal learning ID returned from `add` (greppability is the point). If the learning is workflow-only and has no single source-code anchor, skip the citation — push-injection covers it.

   **Hard prohibition.** Never add the citation inside `crates/**/assets/**` (skill files, prompt assets, any shipped plugin asset) or other consumer-facing surfaces. Workspace-local artifact IDs become dangling references in other workspaces — this is the distribution-boundary rule for workspace-local artifact IDs. For guidance at those surfaces, the push-injected learning *is* the delivery mechanism.

4. **List to audit.** `orbit learning list --status active` returns envelope-only records ordered by `updated_at desc`. Filter by `--tag` or `--path` to narrow. Use `orbit learning show --id <ID>` to inspect the full body and evidence.

5. **Update vs supersede:**
   - **Update** when the learning is still substantively the same and you are refining the wording, narrowing the scope, or attaching new evidence. `update` *replaces* `scope` and `evidence` (it does not merge) — pass the full new arrays.
   - **Supersede** when the guidance has materially changed: the new advice contradicts or significantly extends the old one. `supersede` writes both pointers atomically (`old.superseded_by = new.id`, `new.supersedes = old.id`) and excludes the old record from default search.
   - `update` is rejected on already-superseded records — use `supersede` to chain another replacement.

6. **Prune for stale.** Run `orbit learning prune --stale-only` periodically to surface learnings whose `scope.paths` no longer resolve to any tracked file (per the `§7.3` staleness rules in the design doc). Combine with `--delete` to archive flagged records by flipping their status to `superseded` with `superseded_by: null` — only do this after reading the candidates and deciding none are still load-bearing.

7. **Sync when YAML is touched out-of-band.** YAML under `.orbit/learnings/` is the source of truth; SQLite is a rebuildable envelope index. If a merge, branch switch, or external script edits the YAML directly, a human/operator can run the CLI-only `orbit learning sync` to re-sync the index — otherwise `list` and `search` will return stale results.

## Operating Rules

- **Never edit `.orbit/learnings/<id>/learning.yaml` directly.** All writes go through the tools so envelope cache, supersede pointers, and audit events stay consistent.
- **Stage new learning artifacts from the current worktree.** `add` creates
  `learning.yaml`, `votes.jsonl`, and `comments.jsonl` under
  `.orbit/learnings/L-NNNN/` in the worktree where you ran the tool; commit
  those files with the branch that needs the guidance.
- **Use comments for footnotes, not rewrites.** `orbit.learning.comment.add` is for brief observations tied to the current wording of a learning; the body is capped at 500 characters. For corrections, delete the old comment and add a new one. For material guidance changes, create a replacement learning and use `orbit.learning.supersede`.
- **Never invent learning IDs.** `add` allocates them; cite returned IDs verbatim.
- **One learning, one piece of guidance.** If a record needs three sub-points, it is probably three learnings with overlapping scopes — easier to maintain and prune.
- **Keep `summary` ≤ 280 characters and write it as a directive** (e.g. *"Always X before Y in <crate>"*, not *"Notes about X"*). Push-injection surfaces the summary first; agents skim it for relevance in milliseconds.
- **`scope` is OR semantics**, not AND. A learning with `paths: ["crates/A/**"], tags: ["security"]` fires for *every* file under `crates/A/` *and* for every prompt tagged `security`. Splitting concerns into separate learnings is usually clearer than over-broadening one.
- **Evidence is not optional in spirit.** If a learning has no `evidence`, you should be able to name the human conversation that produced it; if you cannot, the learning is probably a hunch, not a learning. Add `evidence: [{ kind: "external", ref: "<conversation-link-or-note>" }]` rather than omitting.
- **Push-injection happens automatically.** This skill does not invoke push — Orbit's runtime does that whenever an agent touches matching scope. Authoring quality directly determines injection quality.
- **`update` replaces, does not merge.** When changing one field, re-pass the unchanged values for `scope` and `evidence`, or you will silently wipe them.
- **Never `update` a superseded record.** The tool rejects it; `supersede` again instead.
- **Use `prune --stale-only` to read first**, `--delete` only after auditing. Default to read-only.
- **`reindex` is a recovery / migration op**, not part of normal flow. Most operators never need it.

## Minimal Commands

Author a path-scoped learning with a body and a task evidence link:

```bash
# /tmp/learning.md contains the long-form body
orbit learning add \
  --summary "Always run `make fmt` before committing under crates/orbit-cli — clippy fails on stray spacing" \
  --path "crates/orbit-cli/**/*.rs" \
  --tag rust \
  --tag formatting \
  --body-file /tmp/learning.md \
  --evidence task:T20260514-3 \
  --priority 100 \
  --json
```

Find what would inject for a specific file:

```bash
orbit learning list --path crates/orbit-cli/src/command/learning/add.rs
```

Attach a brief observation to an existing active learning:

```bash
orbit learning comment add \
  --learning-id L-0001 \
  --body "When this fires in orbit-core, also check the MCP safe surface." \
  --model codex \
  --json
orbit learning comment list --learning-id L-0001
orbit learning comment delete --id C20260514-1
```

Replace one learning with another:

```bash
orbit learning supersede --id L-0001 --with L-0009
```

Audit stale learnings without deleting:

```bash
orbit learning prune --stale-only
```

## Common Mistakes

| Mistake | Why it fails | Correct form |
|---------|--------------|--------------|
| Hand-writing `.orbit/learnings/<id>/learning.yaml` | Skips envelope index update and audit attribution | Use `orbit.learning.add` / `update` / `supersede` |
| Editing a comment in place | Comments are append-only audit records | Delete the old comment and add a corrected one |
| Commenting on a superseded learning | Superseded wording is retired from push-injection | Add the comment to the active replacement, or supersede again for content changes |
| Creating a duplicate without checking first | Two records with overlapping scope inject twice and contradict each other | `orbit learning list --path/--tag` (and `orbit search --kind learning`) before `add` |
| `update` to "fix" a fundamental change in advice | Loses the supersede chain; readers cannot see the old guidance was reversed | `orbit learning supersede --id <old> --with <new>` |
| Calling `update` on a superseded record | Tool rejects with a typed error | `supersede` from the head of the chain instead |
| `scope` with no `paths` and no `tags` | Never injects — record is invisible to push | Include at least one `path` glob or one `tag` |
| Editing YAML directly to "quickly tweak wording" | Index goes stale; next `search` returns old envelope | Use `update`; if YAML must be touched, run CLI-only `orbit learning sync` after |
| Treating `priority` as importance | It is the secondary search-ranking key, not a tier | Leave unset unless tuning ranking |

## Exit Criteria

The learning artifact exists or is updated through `orbit.learning.*`, has a directive `summary`, has at least one `paths` or `tags` entry in `scope`, carries evidence when one exists, and can be retrieved with `orbit learning show --id <ID>`. Stale or contradicted predecessors are explicitly superseded, not silently overwritten. When the learning has a code anchor, a citation comment at the source location ships alongside the learning.
