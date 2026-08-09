---
name: orbit-knowledge
description: Author, update, supersede, prune, and audit Orbit's two durable-decision primitives — project learnings (scope-injected guardrails) and ADRs. Use before adding or renaming a `## ADR-` heading by hand, since IDs are allocated globally.
---

# Orbit Knowledge

Two sibling primitives, one lifecycle discipline.

**Learnings** are push-injected: when an agent touches a file, dir, or workflow matching a learning's `scope`, it lands in that agent's prompt automatically. This skill is the pull side — author, find, replace, archive.

**ADRs** record a decision that had a real alternative, constrains future work, and is costly to reverse. Stricter lifecycle, globally allocated IDs.

Both surfaces (MCP `orbit_learning_*` / `orbit_adr_*`, CLI `orbit tool run orbit.learning.*` / `orbit.adr.*`) take identical JSON and need `model`. Prefer `--body-file` / `--input-file` over inline JSON so multi-line markdown survives shell quoting.

## Rules for both

- **Never hand-edit `.orbit/learnings/<id>/learning.yaml` or `.orbit/adrs/<status>/<id>/{adr.yaml,body.md}`.** Writes go through the tools so the envelope cache, supersede pointers, and audit events stay consistent.
- **Never invent an ID.** `add` allocates globally; cite the returned ID verbatim.
- **Search before adding.** Two overlapping-scope records inject twice and contradict each other.
- **Update replaces, it does not merge** — re-pass unchanged fields (`scope`/`evidence` for learnings, the full body for ADRs) or you silently wipe them. Use `supersede` when the guidance or decision *materially changes*: it writes both pointers atomically and drops the old record from default search. `update` is rejected on an already-superseded record; supersede again instead. There is no comment or vote surface — corrections go through `update`/`supersede`, provenance through `evidence`.
- **Stage new artifact files from the current worktree**, alongside the change that motivated them. Sibling worktrees see remote stubs until those body files are locally readable.
- **Citation-at-anchor.** When a record encodes a convention enforced at a specific code site, drop a one-line citation there (`// L-NNNN: <rationale>`). **Never** put such a citation in a shipped instruction or prompt asset — skill files, prompt templates, bundled plugin assets, anything served to other workspaces — because workspace-local IDs are dangling references there. At those surfaces the push-injected learning, or an ADR's `## Consequences` sentence, is the delivery mechanism.

## Learnings

| Workflow | MCP | CLI |
|------|-----|-----|
| Add | `orbit_learning_add({...})` | `orbit learning add --summary "..." --path "src/**/*.rs" --tag rust --body-file note.md` |
| Search | `orbit_search({...})` | `orbit search --kind learning <text>` (`--hybrid` after `orbit semantic index --kind learnings`) |
| Show / update / supersede | `orbit_learning_show/update/supersede({...})` | `orbit learning show L-NNNN` · `update --id L-NNNN --priority 200` · `supersede --id L-NNNN --with L-MMMM` |
| List, prune, sync | CLI-only | `orbit learning list --status active --tag rust [--path <glob>]` · `prune --stale-only [--delete]` · `sync` |

`scope: { paths?, tags? }` is **OR** semantics — it fires on *any* matching path glob or *any* matching tag. Split concerns into separate learnings rather than over-broadening one, and note that a scope with neither `paths` nor `tags` never injects at all.

Write `summary` as a directive ("Always X before Y in `<crate>`"), ≤280 chars, since push-injection surfaces it first. Attach `evidence: [{kind: "task"|"commit"|"external", ref: "..."}]` whenever the learning came from a real incident, PR, or task — one you can't cite a source for is a hunch. `priority` (0–255) is a secondary search-ranking key, not an importance badge.

`prune --stale-only` surfaces learnings whose `scope.paths` no longer resolve; read them before `--delete`. `sync` re-syncs the SQLite envelope index when YAML was touched out-of-band by a merge or branch switch — YAML is the source of truth.

Legacy `L<YYYYMMDD>-N` IDs were migrated and should now appear only in `legacy_ids`; `L-NNNN` is canonical.

Exit: the learning lives in `orbit.learning.*`, has a directive summary, at least one `paths` or `tags` entry, evidence where it exists, and reads back via `orbit learning show <ID>`. A code-anchored learning ships its citation in the same change.

## ADRs

| Tool | MCP | CLI |
|------|-----|-----|
| Add / show / update / supersede | `orbit_adr_add/show/update/supersede({...})` | `orbit tool run orbit.adr.add --input-file adr.json` · `.show --input '{"id":"<id>"}'` · `.update --input-file update.json` · `.supersede --input '{"old_id":"...","new_id":"..."}'` |

Listing is deliberately not on the agent MCP surface — discover with `orbit search --kind adr`. (`orbit tool run orbit.adr.list` remains for CLI admin.)

**If your repo keeps ADR headings inside a docs file and you're about to add or rename a `## ADR-` heading: stop and run `orbit.adr.add` first**, then use the allocated global ID as the heading verbatim. Picking the next sequential local number — or one that merely looks global — produces an orphan decision invisible to `orbit search --kind adr`, `orbit.adr.show`, and legacy-ID resolution. Backfill an existing local-numbered ADR through `orbit.adr.add` with `legacy_ids` set. Where that file lives follows the target repo's own instructions and configured docs roots, not a fixed convention.

**Propose an ADR only when both hold:** a real alternative was on the table, *and* the choice is meaningfully costly to reverse. "Surprising without context" is a strong signal but not a third requirement — lock-in can be real without being surprising (a Postgres or monorepo pick), so weigh it as a tiebreaker. Qualifying shapes: a deliberate deviation from the obvious path; a constraint invisible in the code itself; a non-obvious rejected alternative worth recording; a technology choice with real lock-in. Everything else belongs in a design doc, a spec, or an existing ADR's instance table.

**Workflow.** Inspect nearby decisions first (`orbit search "<concept>" --kind adr`, `orbit.adr.show`, legacy-ID lookup). New decision → `add`; body or metadata correction → `update`; reversal of an accepted ADR → create the replacement, accept it with a related task, then `supersede`. `related_features` are feature/area names, `tags` free-form cross-artifact labels, `paths` repo-relative globs the decision constrains. `related_tasks` may be empty for a speculative *proposed* ADR — don't invent a task to satisfy one — but **acceptance requires a real related task**, added in the same `update` call.

The body needs exactly `## Context`, `## Decision`, `## Consequences`, with at least one consequences bullet starting `Cost:` — the validator rejects new ADRs without it.

```markdown
## Context
<1-3 sentences: what forced a decision, what alternatives were real>
## Decision
<1-3 sentences: what was chosen>
## Consequences
- <observable/operational consequence>
- Cost: <explicit tradeoff future readers need to know>
```

Exit: the ADR lives in `orbit.adr.*`, has a valid body, names relevant features, preserves meaningful legacy aliases, and reads back via `.show`. A code-anchored ADR ships its citation in the same PR.
