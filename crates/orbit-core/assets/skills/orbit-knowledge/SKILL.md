---
name: orbit-knowledge
description: Create, search, update, supersede, prune, or audit Orbit's two durable-decision primitives — project learnings (recurring gotchas, incident root-causes, cross-session guardrails, push-injected by scope) and Architecture Decision Records (ADRs — decisions with a real alternative, accepted/proposed/superseded lifecycle, global ID allocation). Triggers on "learning", "gotcha", "guardrail", "ADR", "decision record", editing an ADR decision-record file, accepting/superseding a decision, or adding a `## ADR-` heading. Covers scope-OR matching, evidence shape, update-vs-supersede (there is no comment or vote surface — corrections go through `update`/`supersede`, provenance through `evidence`), and why to never hand-edit `.orbit/learnings/` or `.orbit/adrs/`.
---

# Orbit Knowledge

Two sibling primitives, one shared lifecycle discipline. **Learnings** are push-injected guidance: when an agent touches a file/dir/workflow matching a learning's `scope`, it's injected into the prompt automatically — this skill is the pull/curate side (author, find, replace, archive). **ADRs** record decisions with a real alternative, a constraint on future work, and non-trivial cost — stricter lifecycle, global ID allocation.

Both surfaces (MCP `orbit_learning_*`/`orbit_adr_*`, CLI `orbit tool run orbit.learning.*`/`orbit.adr.*`) accept identical JSON; always include `model` (agent family: `codex`/`claude`/`gemini`/`grok`). Prefer `--body-file`/`--input-file` over inline JSON so multi-line markdown isn't mangled by shell quoting.

## Shared rules (apply to both)

- **Never hand-edit `.orbit/learnings/<id>/learning.yaml` or `.orbit/adrs/<status>/<id>/{adr.yaml,body.md}`.** All writes go through the tools so envelope cache, supersede pointers, and audit events stay consistent.
- **Never invent an ID.** `add` allocates both learning and ADR IDs globally; cite the returned ID verbatim.
- **Stage new artifact files from the current worktree** alongside the code/doc change that motivated them — sibling worktrees see remote stubs until that worktree's body files are locally readable.
- **Update replaces, does not merge** — re-pass unchanged fields (`scope`/`evidence` for learnings; full body for ADRs) or you'll silently wipe them. **Supersede** when guidance/decision materially changes; it writes both pointers atomically and excludes the old record from default search. `update` is rejected on an already-superseded record — `supersede` again instead.
- **Citation-at-anchor, with a hard prohibition.** When a learning/ADR encodes a convention enforced at a specific code site, drop a one-line citation (`// L-NNNN: <rationale>` or `// <adr-id>: <rationale>`) at that site. **Never** place the citation inside shipped instruction or prompt assets (skill files, prompt templates, bundled plugin assets) or any other consumer-facing surface served to other workspaces — workspace-local artifact IDs become dangling references there. At those surfaces, the push-injected learning (or a `## Consequences` sentence for ADRs) is the delivery mechanism instead.
- **Search before adding**, to avoid duplicate/contradicting records covering the same scope.

## Learnings

| Tool / workflow | MCP | CLI |
|------|-----|-----|
| Add | `orbit_learning_add({...})` | `orbit learning add --summary "..." --path "src/**/*.rs" --tag rust --body-file note.md` |
| Search | `orbit_search({...})` | `orbit search --kind learning <text>` (add `--hybrid` after `orbit semantic index --kind learnings`) |
| Show / update / supersede | `orbit_learning_show/update/supersede({...})` | `orbit learning show L-NNNN` / `update --id L-NNNN --priority 200` / `supersede --id L-NNNN --with L-MMMM` |
| List/audit, prune, sync | CLI-only | `orbit learning list --status active --tag rust [--path <glob>]`, `orbit learning prune --stale-only [--delete]`, `orbit learning sync` |

**Workflow:** (1) search first — `orbit learning list --path/--tag` or `orbit search --kind learning` — prefer `update`/`supersede` over a duplicate. (2) Add with tight `scope: { paths?, tags? }` (OR semantics — fires on *any* path glob OR *any* tag; split concerns into separate learnings rather than over-broadening one). Include `evidence: [{kind: "task"|"commit"|"external", ref: "..."}]` whenever it came from a real incident/PR/task — a learning you can't cite a source for is a hunch. `priority` (0–255) is a secondary search-ranking key, not an importance badge. Keep `summary` ≤280 chars, written as a directive ("Always X before Y in `<crate>`"), since push-injection surfaces it first. (3) `prune --stale-only` periodically to surface learnings whose `scope.paths` no longer resolve; read before `--delete`. (4) `sync` (CLI-only) re-syncs the SQLite envelope index if YAML was touched out-of-band (merge, branch switch) — YAML is the source of truth.

Legacy `L<YYYYMMDD>-N` IDs were migrated and should only appear in `legacy_ids`; canonical format is `L-NNNN`. Corrections to current wording go through `update`; material changes go through `supersede`; provenance for a new observation goes into `evidence`. (The vote and comment surfaces were removed — `priority` + search rank cover ranking, and `update`/`supersede`/`evidence` cover corrections/provenance.)

```bash
orbit learning add --summary "Always run the repo formatter before committing under src/ — the linter fails on stray spacing" \
  --path "src/**/*.rs" --tag lang --tag formatting --body-file /tmp/learning.md \
  --evidence task:<task-id> --priority 100 --json
```

Common mistakes: hand-writing YAML (skips envelope+attribution — use the tools); creating a duplicate without checking first (two overlapping-scope records inject twice and contradict); `update` to "fix" a fundamental change in advice (loses the supersede chain — `supersede` instead); `scope` with no `paths` and no `tags` (never injects).

Exit: the learning exists/updates through `orbit.learning.*`, has a directive `summary`, ≥1 `paths`/`tags` entry, evidence when it exists, and is retrievable via `orbit learning show <ID>`. A code-anchored learning ships its citation in the same change.

## ADRs

| Tool | MCP | CLI |
|------|-----|-----|
| Add / show / update / supersede | `orbit_adr_add/show/update/supersede({...})` | `orbit tool run orbit.adr.add --input-file adr.json` / `.show --input '{"id":"<id>"}'` / `.update --input-file update.json` / `.supersede --input '{"old_id":"...","new_id":"..."}'` |

Listing is **not** on the agent MCP surface — use `orbit search --kind adr` for read-side discovery; CLI/admin retains `orbit tool run orbit.adr.list --input '{"feature":"<feature>"}'` (agents shouldn't call it via MCP). ADRs and docs are sibling indexes: `orbit search --kind all|doc|adr` federates ADR metadata read-only; `--hybrid` applies to `--kind adr` after `orbit semantic index --kind adrs`.

**When your repo keeps ADR headings in a docs file and you edit one directly:** if you're about to add or rename a `## ADR-` heading, **stop and run `orbit.adr.add` first**, then use the allocated global ID as the local heading verbatim. (Where that file lives — its name and location — follows the target repo's own instructions and configured docs roots, not a fixed convention.) Picking the next sequential local number, or a number that merely "looks global," both produce orphan decisions invisible to `orbit search --kind adr` / `orbit.adr.show` / legacy_id resolution. Backfill an existing local-numbered ADR via `orbit.adr.add` and set `legacy_ids`.

**Workflow:** (1) inspect nearby decisions first (`orbit search "<concept>" --kind adr`, `orbit.adr.show`, `legacy_id` lookup for migrated per-feature refs). (2) new decision → `add`; body/metadata correction → `update`; reversal of an accepted ADR → create the replacement, accept it with a related task, then `supersede`. (3) body needs exactly `## Context`, `## Decision`, `## Consequences`, with ≥1 consequences bullet starting `Cost:`. (4) `related_features` = feature/area names; `tags` = free-form cross-artifact labels; `paths` = repo-relative globs the decision constrains. (5) `related_tasks` may be empty for a speculative proposed ADR — don't invent a task just to satisfy one; **acceptance requires a real related task**. (6) verify with `.show` or `orbit search --kind adr`.

Creation is warranted only when all three hold: a real alternative was on the table, the choice constrains future work, and the cost is non-trivial and worth preserving. Otherwise put the detail in a design doc, a spec, or an existing ADR's instance table.

```markdown
## Context
<1-3 sentences: what forced a decision, what alternatives were real>
## Decision
<1-3 sentences: what was chosen>
## Consequences
- <observable/operational consequence>
- Cost: <explicit tradeoff future readers need to know>
```

```bash
orbit tool run orbit.adr.add --input-file /tmp/orbit-adr.json --pretty
# {"title":"...","body":"## Context\n...\n## Decision\n...\n## Consequences\n- ...\n- Cost: ...\n",
#  "owner":"codex","related_features":["task-artifacts"],"related_tasks":[],"tags":[...],"paths":[...],"model":"<agent-family>"}
```

Common mistakes: hand-writing `.orbit/adrs/...` (skips allocation/validation/provenance); creating a task just to propose (proposed ADRs allow empty `related_tasks`); accepting without a real task (acceptance requires one, add it in the same `update` call); omitting `Cost:` (validator rejects new ADRs without it); treating a local heading number as global (use the allocated ID, alias via `legacy_ids`).

Exit: the ADR exists/updates through `orbit.adr.*`, has a valid body, names relevant features, preserves meaningful legacy aliases, and reads back via `.show`. A code-anchored ADR ships its citation in the same PR.
