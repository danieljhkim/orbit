---
summary: "Design Docs — Overview"
type: design
title: "Design Docs — Overview"
owner: claude
last_updated: 2026-05-17
status: Draft
feature: design-docs
doc_role: overview
tags: ["design-docs"]
---

# Design Docs — Overview

The design-docs system is Orbit's convention for capturing architectural intent: every load-bearing feature gets a folder under `docs/design/<feature>/` with four numbered docs (overview, design, vision, decisions) plus `specs/` and `references/` subfolders. [CONVENTIONS.md](../CONVENTIONS.md) is the rulebook. The `orbit design` CLI and the `orbit.design.{init,list,show}` MCP tools make scaffolding and inspection cheap so agents apply the discipline by default rather than skip it.

This document is the entry point. [2_design.md](./2_design.md) specifies layout, frontmatter, ADR rules, the same-PR review gate, and the tool surface. [3_vision.md](./3_vision.md) names open questions (lint enforcement, ADR migration to a queryable artifact, semantic search). [4_decisions.md](./4_decisions.md) records the load-bearing decisions, including the [ORB-00112] removal of the per-PR freshness gate.

---

## 1. Motivation

AI coding agents are fast enough that "write a design doc" feels like a tax — easy to skip, easy to under-fill, easy to let drift after the implementation lands. Three concrete failure modes the convention exists to prevent:

1. **Drift between docs and code.** Without an enforcement loop, design docs describe the system as it was thought about, not as it currently is. Six months later the doc is worse than nothing — it actively misleads. Orbit's active gate is the same-PR update rule in [CLAUDE.md](../../CLAUDE.md): behavior changes that affect a design folder must update that folder in the same review.

2. **Free-form folders that don't read as one system.** When every author picks their own structure, readers cannot navigate. The 4-doc role split (overview / design / vision / decisions) gives a stable contract: open `1_overview.md` for the elevator pitch, `2_design.md` for the current implementation, `3_vision.md` for what's still open, `4_decisions.md` to find why a particular choice was made. Cross-feature reading stays cheap.

3. **ADR rot.** Architecture Decision Records work only if every entry names a real alternative, a forward constraint, and a non-trivial cost ([CONVENTIONS.md §4](../CONVENTIONS.md)). Without rules, the log fills with trivial "we picked X" entries that bury the load-bearing decisions. The 3-of-3 earning rule and the mandatory `Cost:` line keep the signal-to-noise high.

A fourth failure mode is downstream of the others: **agents will not apply a discipline that is expensive to apply.** When scaffolding a new feature requires opening four sibling folders to remember the file names and section orders, agents either skip docs entirely or produce slop. The `orbit.design.init` MCP tool ([2_design.md §5](./2_design.md)) collapses that to one call — the four files exist with correct frontmatter and the standard section skeleton in under a second.

---

## 2. Core Concepts

### 2.1 Feature folder

One per architectural concern. Folder name is lowercase, hyphenated, singular (`knowledge-graph`, `design-docs`, `policy-sandbox`). Contains exactly four numbered markdown docs plus two subfolders:

```
docs/design/<feature>/
├── 1_overview.md       # what and why
├── 2_design.md         # current implementation
├── 3_vision.md         # forward-looking
├── 4_decisions.md      # ADR log
├── specs/              # optional mechanism specs
└── references/         # optional glossary + lookups
```

Twelve folders existed as of 2026-05; this folder is the thirteenth. The layout is enforced by cross-review and (eventually) lint. See [CONVENTIONS.md §1](../CONVENTIONS.md).

### 2.2 Numbered docs and required sections

Each numbered file has a fixed role and required section list — see [CONVENTIONS.md §3](../CONVENTIONS.md) for the full table. Roles in one line each:

- `1_overview.md` — elevator paragraph, motivation, core concepts, an "At a Glance" table mapping concerns to files and tasks.
- `2_design.md` — scope paragraph, then numbered mechanism sections describing the current implementation, ending with a mandatory `Concerns & Honest Limitations` section.
- `3_vision.md` — open questions (numbered), prior work (categorized subsections), what may be distinctive, references.
- `4_decisions.md` — append-only ADR log.

Every numbered doc ends with a `Task References` section listing the task IDs cited in that doc.

### 2.3 Required frontmatter and the `last_updated` anchor

```yaml
---
title: <Feature> — <Doc Role>
owner: <agent family — codex, claude, grok, or gemini>
last_updated: YYYY-MM-DD
status: <Draft | Accepted>
feature: <feature-slug — matches the folder name>
doc_role: <overview | design | vision | decisions>
---

# <Feature> — <Doc Role>
```

`last_updated` is the author-maintained freshness anchor. The author updates it manually whenever the doc body changes; trivial reformat commits should not reset the freshness clock. See [4_decisions.md ADR-0159](./4_decisions.md) for why. The full frontmatter schema is defined in [`CONVENTIONS.md §2`](../CONVENTIONS.md).

### 2.4 ADR (Architecture Decision Record)

A single decision in `4_decisions.md`. Strict template ([CONVENTIONS.md §4](../CONVENTIONS.md)):

```
## ADR-NNN — <short title>

**Status:** <Accepted | Proposed | Superseded by ADR-MMM> · YYYY-MM · [T...]

**Context.** <why a decision was forced>

**Decision.** <what we chose>

**Consequences.**
- <bullet>
- Cost: <explicit tradeoff — every ADR must name at least one cost>
```

A decision earns an ADR only if **all three** hold: (1) a real alternative was on the table, (2) the choice constrains future work, (3) the cost is non-trivial and not inferable from the decision itself. If only one or two hold, the decision belongs in `2_design.md` prose. See [4_decisions.md ADR-003](./4_decisions.md).

### 2.5 Tool surface

CLI: `orbit design init`, `orbit design list`, and `orbit design show` expose the same feature-folder operations as the MCP tools for local operators.

MCP: `orbit.design.init`, `orbit.design.list`, and `orbit.design.show` let agents scaffold and inspect design folders without hand-creating files. See [2_design.md §5](./2_design.md).

### 2.6 Convention enforcement

[CONVENTIONS.md](../CONVENTIONS.md) is the rulebook. Today, enforcement is cross-review: when one agent reviews another's design folder, the reviewer treats CONVENTIONS.md and the same-PR update rule as the checklist. Two enforcement gaps remain open: per-doc lint of required frontmatter + Task References section, and per-ADR lint of the Cost line. See [3_vision.md §1](./3_vision.md).

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Convention rulebook (folder layout, frontmatter, sections, ADR template, glossary) | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| `orbit design` CLI (`init`, `list`, `show`) | [crates/orbit-cli/src/command/design.rs](../../../crates/orbit-cli/src/command/design.rs) | [ORB-00019], [ORB-00112] |
| `orbit.design.{init,list,show}` MCP tool registry | [crates/orbit-tools/src/builtin/orbit/design/](../../../crates/orbit-tools/src/builtin/orbit/design/) | [ORB-00019], [ORB-00112] |
| MCP dispatch into core | [crates/orbit-core/src/runtime/orbit_tool_host/design_tools.rs](../../../crates/orbit-core/src/runtime/orbit_tool_host/design_tools.rs) | [ORB-00019], [ORB-00112] |
| Existing feature folders (12) | [docs/design/](../) | [ORB-00006] |
| Adjacent ADR-artifact proposal (would replace `4_decisions.md` with a queryable store) | [docs/design/adr-artifact/](../adr-artifact/) | — |
| Glossary | [references/glossary.md](./references/glossary.md) | — |

---

## Task References

- [ORB-00006] — Refreshed ARCHITECTURE.md and the existing design folders to a consistent layout that became the basis for CONVENTIONS.md.
- [ORB-00019] — Promoted design-doc scaffolding and inspection to first-class `orbit design` CLI + `orbit.design.*` MCP tools.
- [ORB-00090] — Aligned design-doc owner examples with the agent-family identity convention.
- [ORB-00112] — Removed the per-PR freshness gate and kept `init` / `list` / `show` as the useful design-doc tooling.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
