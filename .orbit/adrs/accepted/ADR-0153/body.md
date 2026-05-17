## Context

The ADR v2 surface (`orbit.adr.*` tools + `.orbit/adrs/` store) shipped, but `docs/design/CONVENTIONS.md` §4 was not updated to require its use. As a result, agents editing `docs/design/<feature>/4_decisions.md` continued to follow the v1 markdown template and authored new local `## ADR-NNN` headings without allocating a global ID. The most recent example is `project-learnings/4_decisions.md` ADR-006 from ORB-00095; the global corpus does not know that decision exists. [ORB-00019] explicitly declined to settle the v1/v2 boundary (“do NOT collapse them; file a separate ADR”) — ORB-00098 is that follow-up.

Three policy options were on the table:

1. **Full v2 cutover.** New ADRs go *only* through `orbit.adr.add`; per-feature `4_decisions.md` becomes auto-generated from the store via the generator described in `docs/design/adr-artifact/2_design.md` §7.5. Hand-editing the markdown is disallowed.
2. **Dual surface with required mirroring (`legacy_ids` keyed on local 3-digit ADR-NNN).** Agents author the markdown ADR with a local 3-digit heading and also call `orbit.adr.add` with `legacy_ids: ["<feature>/ADR-NNN"]` in the same change.
3. **Markdown-first with sync tool.** Agents author markdown; a future `orbit adr sync` tool ingests new local ADRs into the store automatically.

Option 1 is the long-term destination but is not shippable in one task: the markdown generator does not exist, and retiring hand-edited `4_decisions.md` is a substantial behavior change. Option 3 introduces a second source of truth (markdown is canonical until sync runs) and indefinitely defers the global record — it weakens the corpus while pretending to feed it. Option 2 keeps the local 3-digit numbering scheme alive forever as the canonical heading, even though the heading number itself carries no information once a global ID exists.

The existing `docs/design/agent-families/4_decisions.md` already demonstrates a fourth option in the wild: the local heading **is** the global ID (`## ADR-0151 — ...`). It was allocated via `orbit.adr.add` first; the local file holds the long-form narrative and the global record holds metadata. No mirroring bookkeeping, no second numbering scheme.

## Decision

Go with the **global-ID-heading** stance:

- New ADRs MUST be allocated via `orbit.adr.add` *before* the local entry is written.
- The local heading in `docs/design/<feature>/4_decisions.md` uses the allocated global ID verbatim: `## ADR-NNNN — <title>` (4-digit, zero-padded).
- The local entry remains the long-form narrative log; the global record under `.orbit/adrs/` is the source of truth for ID, status, owner, `related_features`, and `related_tasks`.
- Existing local 3-digit headings (`activity-job/ADR-001`–`ADR-036`, etc.) are grandfathered. They may be backfilled opportunistically when a folder is being substantially edited; nothing forces it.
- `project-learnings/4_decisions.md` ADR-001–ADR-006 are backfilled in this task because they are recent and small enough to do cleanly.

This defers full v2 cutover (option 1) without blocking it: when the markdown generator ships, every `4_decisions.md` is already keyed on global IDs, so the generator can reconstruct files from the store without ID rewriting.

Lint enforcement of `[ADR-NNNN]` reference resolution was deliberately deferred from this task. The bet is that updating `CONVENTIONS.md` and the `orbit-adr` / `orbit-design` skill triggers is enough; if drift recurs, the lint becomes a follow-up.

## Consequences

- The local heading number carries information (it's the global ID), so cross-feature references can use the same `[ADR-NNNN]` syntax regardless of which folder the reader is in. No more `(activity-job, ADR-001) ≠ (project-learnings, ADR-001)` ambiguity for new entries.
- Agents have one clear instruction at authoring time: “call `orbit.adr.add` first, write the heading second.” Skill triggers in `orbit-adr` and `orbit-design` are updated to fire on “editing `4_decisions.md`” so the failure mode is caught at the right moment.
- The store is authoritative; `orbit.adr.list`, `orbit.adr.show`, and the future `orbit.adr.search` give honest answers without missing recent decisions.
- Cost: agents must remember the ordering. Without the deferred lint, there is no mechanical gate; the only enforcement is review. If the pattern drifts again, the lint becomes load-bearing.
- Cost: the local 4_decisions.md file ordering is no longer sequential per-folder. Entries are ordered by global ID, which interleaves with every other folder's allocations. Readers who relied on per-folder chronology lose that signal; the `created_at` line in the body preserves it.
- Cost: backfilling `project-learnings/ADR-001`–`ADR-006` rewrites the headings in `docs/design/project-learnings/4_decisions.md`. Existing citations in commits and tasks (e.g. `project-learnings/ADR-001`) still resolve through `legacy_ids`, but plain-text searches over the markdown file lose those numbers.