---
name: orbit-knowledge
description: Author, update, supersede, prune, and audit Orbit's durable knowledge primitives — project learnings (scope-injected guardrails) and feature-local ADR entries in design docs.
---

# Orbit Knowledge

Two sibling primitives, one lifecycle discipline.

**Learnings** are push-injected: when an agent touches a file, dir, or workflow matching a learning's `scope`, it lands in that agent's prompt automatically. This skill is the pull side — author, find, replace, archive.

**ADRs** record a decision that had a real alternative, constrains future work, and is costly to reverse. They are ordinary git-reviewed entries in the repository's designated decision docs.

## Rules for both

- **Never hand-edit `.orbit/learnings/<id>/learning.yaml`.** Learning writes go through the tools so the envelope cache, supersede pointers, and audit events stay consistent.
- **Search before adding.** Two overlapping-scope records inject twice and contradict each other.
- **Learning updates replace, they do not merge** — re-pass unchanged `scope` and `evidence` or you silently wipe them. Use `supersede` when the guidance materially changes.
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

ADRs live in the repository's designated decision docs. Search them through the ordinary docs corpus (`orbit search "<concept>" --kind doc`) and inspect the complete entry in the matching file. There is no `.orbit/adrs/` store, `orbit.adr.*` tool family, or `adr` search kind.

IDs are repo-local and append-only. Before adding an entry, search all designated decision-doc headings, choose the next unused `ADR-NNNN`, and never renumber an existing heading. Put a reversal in a new entry and update the older entry's status/supersession metadata in the same change.

**Propose an ADR only when both hold:** a real alternative was on the table, *and* the choice is meaningfully costly to reverse. "Surprising without context" is a strong signal but not a third requirement — lock-in can be real without being surprising (a Postgres or monorepo pick), so weigh it as a tiebreaker. Qualifying shapes: a deliberate deviation from the obvious path; a constraint invisible in the code itself; a non-obvious rejected alternative worth recording; a technology choice with real lock-in. Everything else belongs in a design doc, a spec, or an existing ADR's instance table.

**Workflow.** Inspect nearby decision docs first. New decision → append the complete entry to the owning feature's file; correction → edit that entry; reversal → append the replacement and mark the older entry superseded. Update the design doc's `Last updated` field in the same PR.

Each entry uses the following skeleton and preserves at least one `Cost:` line:

```markdown
## ADR-NNNN — Title
**Status:** Proposed | Accepted | Superseded
**Date:** YYYY-MM-DD
**Related tasks:** ORB-NNNNN

### Context
<1-3 sentences: what forced a decision, what alternatives were real>
### Decision
<1-3 sentences: what was chosen>
### Consequences
- <observable/operational consequence>
- Cost: <explicit tradeoff future readers need to know>
```

Exit: the complete ADR entry lives in the owning feature's decision doc, has status/date/task metadata and a `Cost:` line, and is discoverable with `orbit search --kind doc`. A code-anchored ADR ships its citation in the same PR.
