---
summary: "Design Docs — Design"
type: design
title: "Design Docs — Design"
owner: claude
last_updated: 2026-05-20
status: Draft
feature: design-docs
doc_role: design
tags: ["design-docs"]
---

# Design Docs — Design

This document specifies the design-docs convention as currently shipped: the per-feature folder layout, the required frontmatter and sections per numbered doc, the ADR template and earning rules, the same-PR update gate, and the CLI + MCP tool surface introduced by [ORB-00019]. The convention itself is in [CONVENTIONS.md](../CONVENTIONS.md); this design doc explains the *implementation* of the convention — what lives where in the Rust crates and how the pieces fit together.

---

## 1. Folder Layout per Feature

```
docs/design/<feature>/
├── 1_overview.md       required
├── 2_design.md         required
├── 3_vision.md         required
├── 4_decisions.md      required
├── specs/              required folder; may be empty
│   └── <mechanism>.md  one mechanism per file
└── references/         required folder; may be empty
    └── glossary.md     recommended
```

Folder names are lowercase, hyphenated, and singular (`knowledge-graph`, not `knowledge-graphs`). Folders prefixed with `_` (e.g. `_archive/`) are ignored by `orbit.design.list` — that is the documented retirement path. No top-level narrative files outside the numbered four are permitted; readers should never have to discover a fifth file in the folder. See [CONVENTIONS.md §1](../CONVENTIONS.md), §10, §7.

The four numbered docs and the two subfolders are scaffolded atomically by [`init_feature`](../../../crates/orbit-core/src/command/design.rs) and exposed via the `orbit.design.init` MCP tool ([§5](#5-tool-surface-cli--mcp)).

---

## 2. Required Frontmatter

Every numbered doc opens with:

```
# <Feature> — <Doc Role>

**Status:** <Draft | Accepted>
**Owner:** <agent family — `codex`, `claude`, `gemini`, or `grok`>
**Last updated:** YYYY-MM-DD
```

The four fields are non-optional. Status follows the lifecycle in [CONVENTIONS.md §7](../CONVENTIONS.md): `Draft` for pre-first-review, `Accepted` for reviewed and load-bearing. There is no `Deprecated` status; retired folders move under `_archive/` ([§1](#1-folder-layout-per-feature)).

`Owner` is the *accountable* agent family (one of `codex`, `claude`, `gemini`, or `grok`), not a committer list or full model string. `Last updated:` is the author's freshness assertion for substantive doc changes — see [4_decisions.md ADR-0159](./4_decisions.md) for why this overrides `git log` of the doc itself.

The `init_feature` scaffold writes today's date and the caller's identity into all four frontmatter blocks at scaffold time; the `orbit.design.show` and `orbit.design.list` tools surface the parsed values ([§5](#5-tool-surface-cli--mcp)).

---

## 3. Required Sections per Numbered Doc

| File | Required sections (in order) |
|------|------------------------------|
| `1_overview.md` | Elevator paragraph · §1 Motivation · §2 Core Concepts · §3 At a Glance (table: concern → file → task) · Task References |
| `2_design.md` | Scope paragraph · numbered mechanism sections (variable count) · §N Concerns & Honest Limitations (mandatory last) · Task References |
| `3_vision.md` | Scope paragraph · §1 Open Questions (numbered) · §2 Prior Work (subsections by category) · §3 What May Be Distinctive · §4 References (Orbit-internal + External) · Task References |
| `4_decisions.md` | Format explainer · ADR entries in ascending number order · Task References |

Every numbered doc ends with a `## Task References` block listing only the task IDs cited in that doc, plus the line `> Resolve any task above with orbit task show <ID> or git log --grep=<ID>.`. See [CONVENTIONS.md §3](../CONVENTIONS.md) for the canonical table.

The role split is the contract that makes cross-feature reading cheap. Authors must not invent new top-level files (`README.md`, `roadmap.md`, `tutorial.md`); the anti-pattern table in [CONVENTIONS.md §10](../CONVENTIONS.md) lists what gets rejected at review.

---

## 4. ADR Template and Earning Rules

The ADR body in `4_decisions.md` is strict — see [CONVENTIONS.md §4](../CONVENTIONS.md) for the canonical template. Two rules carry most of the weight:

**Earning rule (3-of-3).** A decision becomes an ADR only when *all three* hold:

1. **Real alternative.** A different choice was on the table and would have produced a materially different design.
2. **Forward constraint.** The decision shapes future work or rules out a class of approaches.
3. **Non-trivial cost.** The cost line names something a reader could not infer from the decision itself.

If only one or two hold, the decision lives in `2_design.md` prose, as a row in an existing ADR's table, or as a task-ID citation on the parent ADR's status line. The 3-of-3 rule is the only thing keeping the log from filling with trivial entries; cross-review enforces it.

**Cost line rule.** Every ADR must include at least one bullet under `Consequences` labeled `Cost: …`. No cost = the decision was not real. Lint enforcement is open ([3_vision.md §1.2](./3_vision.md)); today the rule is enforced by review.

Numbers are append-only. Superseded entries stay in place with status updated to `Superseded by ADR-MMM`. When a cluster of accepted ADRs all instantiate the same underlying decision, [CONVENTIONS.md §4a](../CONVENTIONS.md) defines a rollup-fold maintenance operation; folded entries keep their numbers with `Status: Superseded by ADR-NNN (folded)` and a one-line pointer.

---

## 5. Tool Surface (CLI + MCP)

### 5.1 CLI: `orbit design`

```
orbit design init <feature> [--owner <agent-family>] [--workspace <path>]
orbit design list [--workspace <path>]
orbit design show <feature> [--workspace <path>]
```

Defined in [`crates/orbit-cli/src/command/design.rs`](../../../crates/orbit-cli/src/command/design.rs). The CLI is a thin shim over `orbit-core::command::design::{init_feature, list_features, show_feature}`:

- `init` validates the feature name, creates the four numbered docs plus `specs/` and `references/`, and writes owner/date frontmatter.
- `list` prints one row per non-`_`-prefixed feature folder.
- `show` prints the parsed docs, owner, `Last updated:`, and paths for one feature.
- `--workspace <path>` overrides the workspace root (defaults to `cwd`). Used by tests and callers that drive the command from outside the repo root.

### 5.2 MCP tools: `orbit.design.{init,list,show}`

Tool registry definitions in [`crates/orbit-tools/src/builtin/orbit/design/`](../../../crates/orbit-tools/src/builtin/orbit/design/); dispatch into the core in [`crates/orbit-core/src/runtime/orbit_tool_host/design_tools.rs`](../../../crates/orbit-core/src/runtime/orbit_tool_host/design_tools.rs). All three accept `workspace?: string` (defaults to server cwd) and the standard agent/model identity params; specific shapes:

| Tool | Required input | Returns |
|------|----------------|---------|
| `orbit.design.init` | `feature: string`, optional `owner: string` | `DesignFeatureSummary` for the scaffolded folder |
| `orbit.design.list` | — | `Array<DesignFeatureSummary>` (one per non-`_`-prefixed subfolder) |
| `orbit.design.show` | `feature: string` | `DesignFeatureSummary` for the named feature; typed `NotFoundKind::DesignFeature` error if absent |

`DesignFeatureSummary` is `{ feature, docs: { "1_overview.md": { path, owner, last_updated, decay_status }, ... }, specs_path, references_path }`. `decay_status` is `"fresh" | "stale"` per-doc; it is retained for read-side compatibility while the standalone freshness gate is removed.

### 5.3 Init scaffolding behavior

`init_feature` validates the name (lowercase, hyphenated, path-safe — rejects spaces, uppercase, special chars), errors if the folder already exists ([4_decisions.md ADR-006](./4_decisions.md) bans clobber), creates `specs/` and `references/`, and writes the four numbered docs with frontmatter populated from today's date and the caller's identity. The body of each scaffolded doc contains the required section headers as empty placeholders so the author starts from the canonical structure rather than a blank file. CONVENTIONS.md is *not* re-seeded by `init_feature` — it lives once at `docs/design/CONVENTIONS.md` and predates this tooling; downstream repos that want a copy author it by hand.

---

## 6. Concerns & Honest Limitations

- **`Last updated:` is a manual discipline.** The author has to update one field when the doc changes substantively. If they forget, the doc looks fresh until the next reviewer notices. The trade is intentional ([4_decisions.md ADR-0159](./4_decisions.md)) — `git log` on the doc would create false freshness from cosmetic edits — but it does mean the discipline is review-enforced today.
- **Lint coverage is missing for structure.** There is no automated check for "doc is missing required sections," "ADR is missing a Cost line," or "frontmatter is malformed." See [3_vision.md §1.2](./3_vision.md).
- **The 3-of-3 ADR earning rule is judgmental.** Reasonable agents disagree on whether a given decision crosses the bar. There is no automated check; cross-review is the only enforcement, and rejected ADRs occasionally re-litigate the rule itself rather than the substance.
- **No cross-feature decision storage.** A decision that touches three features lives in whichever folder its author opened first. The proposed [adr-artifact](../adr-artifact/) feature would dissolve this by lifting ADRs into a queryable artifact store with N:M `related_features`. Until that ships, cross-feature ADRs are duplicated or silently homed in one folder.
- **The init scaffold writes the same date into all four docs even when only one will be authored today.** This is harmless — the date is the floor, not the ceiling — but means a freshly scaffolded folder shows four `2026-05-14` dates when only `1_overview.md` may be the one being actively shaped. Authors are expected to bump the date on the docs they actively touch.

---

## Task References

- [ORB-00019] — Promoted design-doc scaffolding and inspection to first-class `orbit design` CLI + `orbit.design.*` MCP tools and scaffolded the per-feature folder layout via `orbit.design.init`.
- [ORB-00090] — Aligned the `Owner` field contract with the agent-family identity convention.
- [ORB-00112] — Removed the per-PR freshness gate after it proved to be the wrong signal; kept `init`, `list`, and `show`.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
