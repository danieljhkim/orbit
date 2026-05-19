---
summary: "ADR Artifact — Decisions"
type: design
title: "ADR Artifact — Decisions"
owner: claude
last_updated: 2026-05-10
status: Draft
feature: adr-artifact
doc_role: decisions
tags: ["adr-artifact"]
---

# ADR Artifact — Decisions

Append-only ADR log for the ADR-artifact feature. Each entry follows the template in [CONVENTIONS.md §4](../CONVENTIONS.md). Numbers are append-only; superseded entries stay in place with status updated. Every ADR cites at least one cost.

Note: this file is itself written in the markdown ADR form this proposal seeks to replace. It will be migrated into the artifact store as part of v2's first migration step (see [2_design.md §7](./2_design.md) and §8.2).

In-place amendment policy: ADRs in `Proposed` status may be refined directly before they ship; CONVENTIONS §4's append-only rule applies once an ADR reaches `Accepted`. Amendments name the review or task ID that prompted them on the Status line.

---

## ADR-001 — Defer implementation to v2; v1 ships docs only

**Status:** Proposed · 2026-05

**Context.** The ADR-artifact proposal touches `orbit-common`, `orbit-store`, `orbit-tools`, `orbit-cli`, and the entire `docs/design/*` corpus. Shipping it as a single v1 change would block other work and rush the migration. The existing `4_decisions.md` markdown pattern is functional today; the problems it causes are growth-rate problems, not correctness problems.

**Decision.** v1 ships this folder as docs-only. No `orbit.adr.*` code, no migration tooling, no CONVENTIONS.md change. v2 ships the store, tools, migration, and the convention update as a coordinated sequence of tasks.

**Consequences.**

- The design is captured while context is fresh and can be cross-reviewed before any code lands.
- Until v2 ships, decisions about this feature live in this `4_decisions.md` — recursively in the form the proposal replaces. Acceptable bootstrap cost.
- Other feature folders continue accumulating markdown ADRs that will need migration; the corpus grows in the meantime.
- Cost: the migration sweep gets bigger every week v2 is deferred. The trim-as-you-touch rule from [CLAUDE.md](../../../CLAUDE.md#design-docs) does *not* apply here — leads should not pre-migrate to a store that doesn't exist.

---

## ADR-002 — Global ADR numbering, not per-feature

**Status:** Proposed · 2026-05 (amended 2026-05-10 — codex review feedback)

**Context.** Today's per-feature numbering (`activity-job/ADR-017`, `knowledge-graph/ADR-017`) means the bare string `ADR-017` is ambiguous. Cross-folder reference requires folder qualification, and any cross-feature decision has no natural home for its number. A subtlety surfaced during cross-review: CONVENTIONS §4a rollups fold N source headings into one body. A scalar `legacy_id` cannot represent the alias relationship — migration would either drop folded paths as resolvable IDs or produce body-less artifacts that violate the required body shape.

**Decision.** ADR IDs are globally unique (`ADR-NNNN`, zero-padded). Per-feature paths from existing markdown ADRs are preserved on each artifact as `legacy_ids: array<string>` for historical resolution but are not the primary key. **Rollups carry one `legacy_ids` entry per folded source heading plus the rollup's own source path** — folded headings do not become their own artifacts.

**Consequences.**

- Cross-feature ADRs have one unambiguous ID.
- A bare `[ADR-0042]` citation in any doc resolves without folder context.
- Both rollup-own and folded-heading citations resolve to the same global ID via `orbit.adr.list --legacy-id=...`.
- Migration must allocate fresh IDs and write `legacy_ids` for every existing entry — non-trivial but mechanical.
- Cost: existing references in git history and commit messages (`see ADR-017`) become ambiguous outside their original folder. `orbit.adr.list --legacy-id=activity-job/ADR-017` resolves them, but no plain grep does. Old PRs and code comments don't get rewritten. The array-valued `legacy_ids` is slightly more complex than a scalar field — parsers must handle the N:1 mapping — but the alternative (dropping rollup aliases or producing body-less artifacts) is worse.

---

## ADR-003 — Three lifecycle states; no `rejected` or `withdrawn`

**Status:** Proposed · 2026-05

**Context.** Common ADR systems (RFCs, IETF, Rust) include `withdrawn` or `rejected` states for proposals that were considered and abandoned. The question is whether ADR-artifact needs them.

**Decision.** Three states only: `proposed`, `accepted`, `superseded`. A proposed ADR that won't ship is **deleted** by its owner (file moved under `.orbit/adrs/deleted/` for archaeology). An accepted ADR the team backs out of is **superseded** by a new ADR that explains the reversal.

**Consequences.**

- Tool surface stays small; lifecycle transitions are unambiguous.
- The "we considered X and rejected it" record lives in the *winning* ADR's Context section, not as a separate withdrawn record. Forces the reasoning to live next to the chosen path, which is more useful for future readers.
- Cost: lossy. A speculative proposal an owner deletes is gone from the corpus; if someone later wants to revisit the same idea, the old proposal isn't preserved as a discoverable record. The deleted-folder archaeology is a partial mitigation, not a search-indexed one.

---

## ADR-004 — Markdown body, structured envelope

**Status:** Proposed · 2026-05

**Context.** Two extremes were on the table for ADR content storage: (a) every field structured (`context`, `decision`, `consequences` as separate YAML strings with cost line as its own array entry), enabling queries like *"show every ADR whose Cost mentions latency"*; (b) one big markdown blob with all metadata as filename / index. (a) buys queryability at the cost of write friction; (b) keeps writing easy but defeats the structured-store rationale. A third option — body as a YAML file with named sections (`content.yaml` with `context:` / `decision:` / `consequences:` keys) — was considered and rejected: prose-in-YAML fights multi-line strings, defeats markdown rendering, produces worse `git diff` output, and diverges from `task_store`'s precedent.

**Decision.** Hybrid: envelope YAML (`adr.yaml`) carries structured metadata (id, status, owner, related_features, related_tasks, supersession, timestamps); a sibling markdown file (`body.md`) holds the human prose (Context / Decision / Consequences). The split matches `orbit-store::task_store`'s existing pattern (envelope + plan.md + execution-summary.md).

**Consequences.**

- Metadata queries are fast; body remains comfortable to write and diff.
- The cost-line rule from [CONVENTIONS.md §4](../CONVENTIONS.md) ("every ADR must name at least one cost") becomes a body-parse check rather than a structured-field invariant. The lint runs against `body.md` with a one-line regex (`^- Cost:`).
- Markdown rendering, syntax highlighting, and editor support work without configuration.
- Cost: queries like *"every ADR whose Cost mentions latency"* require FTS5 over the body, not a typed lookup. Acceptable trade-off until corpus size justifies promoting Consequences to structured form. The §1.4 open question in [3_vision.md](./3_vision.md) revisits this.

---

## ADR-005 — Directory-per-ADR layout

**Status:** Proposed · 2026-05

**Context.** Two layouts were on the table for on-disk storage: (a) flat files per status — `.orbit/adrs/accepted/ADR-0042.yaml` and `.orbit/adrs/accepted/ADR-0042.md` as siblings; (b) directory per ADR — `.orbit/adrs/accepted/ADR-0042/{adr.yaml, body.md}`. (a) is simpler at small corpus sizes; (b) matches what `task_store` already does and anticipates future per-ADR attachments.

**Decision.** Directory per ADR. The layout is `.orbit/adrs/<status>/<id>/{adr.yaml, body.md}`, mirroring `task_store`'s `<status>/<yyyy-mm>/<id>/{task.yaml, plan.md, execution-summary.md, artifacts/}`. ADRs do not date-partition since the corpus is smaller and the ID is already monotonic, but the per-ID directory pattern is identical.

**Consequences.**

- Consistent with `task_store`. Agents reading both stores reuse the same mental model.
- Per-ADR attachments (diagrams, supplementary specs, review-thread exports, related-decision graphs) live next to the ADR without changing the storage contract.
- Status-directory listing remains efficient at thousands of entries (subdirectories scale better than thousands of sibling files of the same prefix on common filesystems).
- Cost: one extra directory level for every ADR, and `orbit.adr.add` performs an additional `mkdir`. Negligible at expected corpus sizes (low thousands of ADRs even years out), but it does mean a single ADR is no longer a one-line `cat .orbit/adrs/accepted/ADR-0042.yaml` to inspect from a shell — readers go through `orbit.adr.show` or `cat .orbit/adrs/accepted/ADR-0042/adr.yaml`.

---

## ADR-006 — Auto-generate per-feature `4_decisions.md` index

**Status:** Proposed · 2026-05 (amended 2026-05-10 — codex review feedback)

**Context.** Once ADRs are first-class artifacts, the hand-maintained per-feature `4_decisions.md` file becomes redundant — the same data lives in the store. Two ways to handle it: delete the file entirely (readers query the store), or replace its contents with a generated index built from `orbit.adr.list --feature=<name>`. Deletion is simpler but removes the affordance of "open one file to see every decision for this feature." A subtlety surfaced during cross-review: CONVENTIONS expects ascending ADR order in `4_decisions.md`, but real corpora are not strictly ascending (`activity-job/4_decisions.md` already has ADR-048 before ADR-047). The generator needs a named stable sort, not just "ascending."

**Decision.** Auto-generate. `4_decisions.md` becomes a build artifact, regenerated from the store via `orbit.adr.list --feature=<name> --format=md` (or an equivalent `make` target). The generated file is committed to git so readers without Orbit installed — and reviewers using only a web git host — can still browse it.

**Two named canonical orders apply:**

| Surface | Order |
|---------|-------|
| Generated `4_decisions.md` (per-feature) | Ascending by **legacy feature ADR number** when present (preserves the historical reading order, including pre-existing non-monotonic cases like ADR-048-before-ADR-047), then by **global `ADR-NNNN`** for legacy-less entries (new ADRs added post-migration). |
| Generated `cross-cutting/4_decisions.md` | Ascending by global `ADR-NNNN`. No legacy ordering applies — cross-cutting ADRs are born under the new scheme. |
| `orbit.adr.list` CLI default | Descending by global `ADR-NNNN` (most recent first; standard browsing order). |

**Consequences.**

- Browsable affordance is preserved; the store remains the source of truth.
- Generators handle ordering, filtering, and supersession links uniformly across features.
- Per-feature `4_decisions.md` preserves the *exact* reading order operators are used to — including historical quirks — so migration doesn't shuffle the file under reviewers.
- `4_decisions.md` becomes off-limits for hand-editing; CONVENTIONS.md is updated to mark it as generated and the migration task adds a CI check that fails on hand-edits.
- Cost: a generated file in git means every ADR add/update produces diff noise. Generation must be **idempotent** — repeated runs against the same store state must produce byte-identical output, or every commit produces spurious churn. The migration tool implements canonical ordering and stable timestamp formatting from day one. Two named orders also means the generator carries two sort implementations and tests for both.

---

## ADR-007 — Cross-cutting ADRs use a dedicated `cross-cutting` index

**Status:** Proposed · 2026-05

**Context.** Some decisions don't belong to a single feature. CLAUDE.md today carries many of them: error-handling conventions, async-locking rules, design-doc reading discipline. Three options for homing cross-cutting ADRs: (a) duplicate them across every relevant feature folder (rot risk); (b) pick one folder arbitrarily — "first in `related_features` wins" (arbitrary, fragile); (c) introduce a dedicated `cross-cutting` pseudo-feature with its own index.

**Decision.** Option (c). `docs/design/cross-cutting/` exists as a pseudo-feature folder. Its generated `4_decisions.md` lists every ADR with `cross-cutting` in `related_features`. Per-feature indexes also include cross-cutting ADRs that touch their feature (via the existing `--feature` filter, which matches any element of `related_features`). The folder holds only `1_overview.md` (short description of what cross-cutting means) and the generated `4_decisions.md` — no `2_design.md` or `3_vision.md`, since the folder describes a class of decisions, not a feature.

**Consequences.**

- Cross-cutting decisions have a canonical home with no duplication.
- Feature folder indexes still show cross-cutting decisions that touch them, so readers don't need to remember to also check `cross-cutting/`.
- CLAUDE.md rules that earn ADR status (after the §1.2 follow-up sweep) migrate into this folder over time. CLAUDE.md remains the high-density rules summary; cross-cutting ADRs are the durable record behind each rule.
- Cost: `docs/design/cross-cutting/` doesn't follow the standard four-numbered-doc layout. CONVENTIONS.md §3 gains a documented exception for pseudo-features, or a small dedicated section. This is the kind of one-off carve-out that conventions docs accumulate; the alternative (forcing a vision/design doc onto a folder that has no design) is worse.

---

## ADR-008 — ADR creation does not require task linkage

**Status:** Proposed · 2026-05

**Context.** `orbit.adr.add` could enforce non-empty `related_tasks` at creation, ensuring every ADR has implementation behind it. Alternatively, it can accept empty `related_tasks` and only require task IDs at the `proposed → accepted` transition (per ADR-001's lifecycle rule). The stricter version prevents speculative ADRs that never ship; the looser version respects the natural workflow where decisions get written down before tasks are filed.

**Decision.** Empty `related_tasks` at creation is permitted. The task requirement applies only to the `proposed → accepted` transition. Keep the surface simple, see how the corpus behaves in practice, tighten later if proliferation becomes a real problem.

**Consequences.**

- Design exploration can produce a proposed ADR before its task is filed — a common workflow ("write down the decision while it's fresh, file the task to land it tomorrow").
- Lifecycle still enforces task linkage at the transition that matters (acceptance), so the corpus doesn't accept untied decisions.
- The "iterate before constraining" framing is a deliberate signal: this rule is the most likely to be reconsidered if behavior on the corpus suggests it should.
- Cost: corpus may accumulate proposed-but-never-shipped ADRs that never get cleaned up. No automated GC; reliance on owner discipline (via the lead-responsibility rule in CLAUDE.md). The §1.3 follow-up in [3_vision.md](./3_vision.md) tracks the revisit.

---

## ADR-009 — Review threads on ADRs

**Status:** Proposed · 2026-05

**Context.** ADRs as v2-ships-them have no formal review surface. Comments on a proposed ADR happen informally — in the related task's review threads, in PR discussion, or in chat. Tasks already have a structured review-thread surface (`orbit.task.review_thread.*`); the question is whether ADRs warrant the same.

**Decision.** Yes. ADRs get `orbit.adr.review_thread.{add, list, reply, resolve}`, mirroring the task surface. Threads are scoped to a single ADR by `adr_id`. Whether the `proposed → accepted` transition should require all threads be resolved is a follow-up question deferred until the surface has real use — for now, the transition does not block on thread state.

**Consequences.**

- Reviewers comment in a structured surface specific to the decision being reviewed, not buried in a task that may cover several decisions.
- Resolution state is queryable: `orbit.adr.review_thread.list --status=open` surfaces unresolved feedback across the corpus.
- Schema and tool surface grow to accommodate threads. Storage attaches threads under the per-ADR directory (`.orbit/adrs/<status>/<id>/review_threads/`), keeping the directory-per-ADR pattern intact.
- Cost: four new tools to maintain and document; risk of duplicating discussion across both task-level and ADR-level review threads when a task implements exactly one ADR. Mitigation: CONVENTIONS.md guidance at v2 ship time — ADR review threads for *the decision*, task review threads for *the implementation*. If duplication remains a real problem post-ship, consider auto-linking the two thread surfaces.

---

## ADR-010 — `orbit.adr.search` lives in `orbit-embed`, registered into `orbit-tools`

**Status:** Proposed · 2026-05

**Context.** [2_design.md §4.6](./2_design.md) routes `orbit.adr.search` through `orbit-embed::vector::VectorStore`. The initial design placed all `orbit.adr.*` tools in `orbit-tools`. Codex flagged the contradiction: `orbit-tools` does not depend on `orbit-embed` (CLAUDE.md architecture diagram), and adding that edge widens the dep graph for one tool. `orbit-embed` already exposes its own `commands::*` surface (`install`, `uninstall`, `reindex`, `stats`) for embedding-adjacent operations.

**Decision.** Split the placement. `orbit.adr.{add, show, list, update, supersede}` and the review-thread tools live in `orbit-tools` (no `orbit-embed` dep needed). `orbit.adr.search` lives in `orbit-embed::commands` alongside the existing embedding-related commands, and is registered into the central tool registry from there. `orbit-tools` stays at its current dep set: `orbit-common`, `orbit-exec`, `orbit-knowledge`, `orbit-policy`.

**Consequences.**

- Crate architecture stays intact. No new edges in the dep graph.
- Search-specific code lives next to the rest of `orbit-embed`'s embedding surface, where the maintainers of `orbit-embed` already operate.
- The tool registry already supports multi-crate registration (tools from `orbit-tools` and `orbit-knowledge` already register through the same surface), so no new infrastructure.
- Cost: `orbit.adr.*` tools are split across two crates instead of co-located. A reader looking for "where is `orbit.adr.add` implemented?" finds it in `orbit-tools`; "where is `orbit.adr.search`?" finds it in `orbit-embed`. The split is principled (dep graph) but does require a doc-comment pointer in each crate so the relationship isn't surprising. Alternative — adding `orbit-embed` as an `orbit-tools` dependency — was rejected because it widens the dep graph permanently for one tool's worth of work.

---

## ADR-011 — Lenient migration mode as default

**Status:** Proposed · 2026-05

**Context.** The artifact schema preserves CONVENTIONS §4's strict requirements: every ADR must have Context, Decision, Consequences, and at least one labeled `Cost:` line. Cross-review revealed that the existing `activity-job` corpus already violates these in `Accepted` entries — ADR-042 has no Consequences section, and ADR-044, -047, -048 have Consequences without a labeled Cost. Strict migration would either reject these (blocking the entire migration) or force a rushed pre-migration cleanup task that bundles unrelated fixes under deadline pressure.

**Decision.** Migration runs in **lenient mode by default**. Entries failing the strict rules are imported with `validation_warnings` recorded on the artifact, and listed in `migration-report.md` for owner follow-up. The strict rules still apply to *new* ADRs created via `orbit.adr.add` after migration; existing entries are grandfathered with a `legacy_validation: warned` flag that the validator treats as a permitted exception until follow-up tasks remediate.

**Consequences.**

- Migration ships without being blocked by corpus debt accumulated over the past year of activity-job work.
- Owners get a concrete punch list (`migration-report.md`) instead of vague "clean up your ADRs" guidance.
- The strict validator's signal stays clean for new work — strict-mode rejects anything new that lacks a Cost line, so the bar holds going forward.
- Cost: known corpus gaps remain in place until owners file remediation tasks. Nothing automatic forces the cleanup. The store accepts incomplete ADRs in perpetuity if no one acts. Mitigation: `orbit.adr.list --validation=warned` is a one-line query that surfaces the debt, and the [lead-responsibility rule](../../../CLAUDE.md#design-docs) makes it the feature lead's job to clear it.

---

## Task References

- [T20260510-27] — Drafted the adr-artifact design folder as a v2 proposal. The original nine ADRs (001–009) are all `Proposed`; each will be flipped to `Accepted` and cite its shipping task ID as v2 implementation work lands.
- [T20260510-28] — Addressed codex P1/P2 review findings: ADR-002 amended in place to cover `legacy_ids` array and rollup aliasing; ADR-006 amended in place with two named canonical orders; ADR-010 added (search tool placement in `orbit-embed`); ADR-011 added (lenient migration mode as default).

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
