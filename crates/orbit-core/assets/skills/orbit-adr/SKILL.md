---
name: orbit-adr
description: Use this whenever an Architecture Decision Record is being created, updated, accepted, or superseded — *including* when editing a `docs/design/<feature>/4_decisions.md` file and about to add or rename a `## ADR-` heading. The global ID must be allocated via `orbit.adr.add` first; the local heading then uses that ID verbatim. Covers ADR body requirements, global ADR ID allocation, related_features/related_tasks/legacy_ids, lifecycle transitions, and how to avoid editing `.orbit/adrs/` files directly.
---

# Orbit ADR

## Purpose

Create and maintain Orbit ADR artifacts through the registered tool surface. ADRs record decisions, not implementation plans: use them when a choice has a real alternative, constrains future work, and carries a non-trivial cost.

ADRs and orbit-docs are sibling indexes by design. ADRs keep their stricter lifecycle and dedicated allocation through `orbit.adr.*`; `orbit search --kind all` (and `--kind adr`) surfaces ADR metadata read-only alongside doc results. For the boundary rationale, run `orbit tool run orbit.adr.list --input '{"feature":"orbit-adr"}'` and inspect the accepted ADR covering the sibling-index search overlay.

ADR artifact files are written into the current worktree's `.orbit/adrs/...`
subtree, while IDs are allocated globally through the shared allocator. Stage
new ADR files from your task worktree alongside the code/doc change that
motivated them; sibling worktrees will see remote stubs until that worktree's
body files are locally readable.

## Tool Invocation

Both surfaces accept the same JSON. Use the CLI examples below when shell access is available; use the MCP names when the Orbit plugin exposes them.

| Tool | MCP | CLI |
|------|-----|-----|
| `orbit.adr.add` | `orbit_adr_add({...})` | `orbit tool run orbit.adr.add --input-file adr.json` |
| `orbit.adr.show` | `orbit_adr_show({...})` | `orbit tool run orbit.adr.show --input '{"id":"<adr-id>"}'` |
| `orbit.adr.list` | `orbit_adr_list({...})` | `orbit tool run orbit.adr.list --input '{"feature":"task-artifacts"}'` |
| `orbit.adr.update` | `orbit_adr_update({...})` | `orbit tool run orbit.adr.update --input-file update.json` |
| `orbit.adr.supersede` | `orbit_adr_supersede({...})` | `orbit tool run orbit.adr.supersede --input '{"old_id":"<old-adr-id>","new_id":"<new-adr-id>"}'` |

Always include `model` in JSON inputs when the tool accepts it. The value is your agent family (`codex`, `claude`, `gemini`, or `grok`); full model strings are accepted and auto-normalized, but the family is canonical. Prefer `--input-file` for `add` and body-changing `update` calls so markdown does not get mangled by shell quoting.

Run `orbit tool show orbit.adr.add` or `orbit tool list` instead of guessing if the local tool surface has changed. Do not assume future surfaces such as `orbit.adr.search` or `orbit.adr.review_thread.*` exist unless `orbit tool list` shows them.

## When Editing `4_decisions.md` Directly

If you are in the middle of writing prose into `docs/design/<feature>/4_decisions.md` and about to add an ADR heading, **stop and run `orbit.adr.add` first**. Then use the allocated global ID as the local heading verbatim. To find the accepted boundary decision behind this rule, run `orbit tool run orbit.adr.list --input '{"feature":"orbit-adr"}'`.

Anti-patterns this rule prevents:

- Picking the next sequential local number (`## ADR-006 — ...` after `ADR-005`) without an allocation.
- Picking a four-digit number that "looks global" without an allocation is worse than the 3-digit version because readers assume `orbit.adr.show` will return that decision; it will not.

Both produce orphan decisions invisible to `orbit.adr.list`, `orbit.adr.show`, and the legacy_id resolution path. If you find an existing local-numbered ADR that was authored this way, backfill it via `orbit.adr.add` and set `legacy_ids` on the resulting record.

## Workflow

1. Inspect nearby decisions before adding a new one.
   - `orbit tool run orbit.adr.list --input '{"feature":"<feature>","model":"<agent-family>"}'`
  - `orbit tool run orbit.adr.show --input '{"id":"<adr-id>","model":"<agent-family>"}'`
   - Use `legacy_id` lookup for migrated per-feature references, for example `{"legacy_id":"activity-job/ADR-039"}`.
2. Decide whether this is a new ADR, an update to a proposed ADR, or a supersession.
   - New decision: `orbit.adr.add`.
   - Body/metadata correction: `orbit.adr.update`.
   - Reversal or replacement of an accepted ADR: create the replacement, accept it with a related task, then `orbit.adr.supersede`.
3. Write the body with exactly the required sections: `## Context`, `## Decision`, `## Consequences`.
4. Include at least one consequences bullet starting with `Cost:`.
5. Set `related_features` to feature folder names such as `task-artifacts`, `activity-job`, or `policy-sandbox`.
6. Leave `related_tasks` empty for speculative proposed ADRs when no task exists yet. Do not create or invent a task just to satisfy an ADR proposal. Acceptance requires a real related task.
7. **Close the loop with a source citation when the ADR has a code anchor.** If the ADR encodes a constraint enforced at a small set of code sites — a `ToolParam` requiring a field, a validation check, a guarded code path — drop a one-line citation comment at each enforcement site in the Rust source so the next reader of that line sees the rationale before they reason their way to weakening it:

   ```rust
   // <adr-id>: <one-line rationale>
   ```

   Use the literal ADR ID returned from `orbit.adr.add` (greppability is the point). If the constraint has no single anchor — pure architectural decision, cross-cutting style — record this in the `## Consequences` body as a single sentence (e.g. "No single code anchor; convention enforced via review.") and skip the citation step.

   **Hard prohibition.** Never add the citation inside `crates/**/assets/**` (skill files, prompt assets, any shipped plugin asset) or other consumer-facing surfaces. Workspace-local artifact IDs become dangling references in other workspaces — this is the distribution-boundary rule for workspace-local artifact IDs. For guidance at those surfaces, author a project learning and let push-injection deliver it.
8. Verify with `orbit.adr.show` or `orbit.adr.list`.

## Creation Rules

- Never edit `.orbit/adrs/<status>/<id>/adr.yaml` or `body.md` directly.
- Never invent the global ADR ID. `orbit.adr.add` allocates it.
- Stage newly created ADR files from the current worktree together with the
  implementation they justify; the allocator prevents cross-worktree ID
  collisions, but the body files belong to the branch that created them.
- Create an ADR only when all three are true:
  - A real alternative was on the table.
  - The choice constrains future work.
  - The cost is non-trivial and worth preserving for future readers.
- Put routine implementation detail in `2_design.md`, a spec file, or an existing ADR's instance table instead of creating a new ADR.
- `related_tasks` may be empty at creation. `proposed -> accepted` requires at least one real task ID on the resulting record.
- Use `legacy_ids` only for aliases that already mean something, such as migrated `feature/ADR-NNN` entries or a local per-feature doc entry you just created in the same change.

## Body Template

```markdown
## Context
<1-3 sentences. What forced a decision and what alternatives were real?>

## Decision
<1-3 sentences. What was chosen?>

## Consequences
- <observable or operational consequence>
- <another consequence>
- Cost: <explicit tradeoff that future readers need to know>
```

Keep bodies concise. ADRs are durable memory for the decision, not a full project brief.

## Minimal Commands

Create a proposed ADR:

`/tmp/orbit-adr.json`:

```json
{
  "title": "Short noun phrase",
  "body": "## Context\nWhat forced the decision.\n\n## Decision\nWhat was chosen.\n\n## Consequences\n- What improves or changes.\n- Cost: The real tradeoff.\n",
  "owner": "codex",
  "related_features": ["task-artifacts"],
  "related_tasks": [],
  "model": "<agent-family>"
}
```

```bash
orbit tool run orbit.adr.add --input-file /tmp/orbit-adr.json --pretty
```

Attach a legacy per-feature alias after creation:

```bash
orbit tool run orbit.adr.update --input '{
  "id": "<adr-id>",
  "legacy_ids": ["<legacy-id>"],
  "model": "<agent-family>"
}'
```

Accept a proposed ADR only once a real task exists:

```bash
orbit tool run orbit.adr.update --input '{
  "id": "<adr-id>",
  "status": "accepted",
  "related_tasks": ["T20260510-28"],
  "model": "<agent-family>"
}'
```

Supersede an old ADR:

```bash
orbit tool run orbit.adr.supersede --input '{
  "old_id": "<old-adr-id>",
  "new_id": "<new-adr-id>",
  "model": "<agent-family>"
}'
```

## Common Mistakes

| Mistake | Why it fails | Correct form |
|---------|--------------|--------------|
| Hand-writing `.orbit/adrs/...` files | Skips allocation, validation, indexes, and provenance | Use `orbit.adr.add` or `orbit.adr.update` |
| Creating a task just to propose an ADR | Proposed ADRs explicitly allow empty `related_tasks` | Leave `related_tasks: []` until acceptance |
| Accepting an ADR without a real task | Lifecycle requires implementation linkage at acceptance | Add the task ID in the same `orbit.adr.update` call |
| Omitting `Cost:` | The validator rejects new ADRs without an explicit cost | Include a consequences bullet beginning `Cost:` |
| Treating a local ADR heading as global | Local per-feature numbers are legacy aliases | Use the allocated global ID and optional `legacy_ids` |

## Exit Criteria

The ADR artifact exists or is updated through `orbit.adr.*`, has a valid body, names the relevant features, preserves any meaningful legacy aliases, and can be read back with `orbit.adr.show`. When the ADR has a code anchor, a citation comment at each enforcement site in the Rust source ships in the same PR.
