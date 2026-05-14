# Design Docs — Overview

**Status:** Draft
**Owner:** claude-opus-4-7
**Last updated:** 2026-05-14

The design-docs system is Orbit's convention for capturing architectural intent: every load-bearing feature gets a folder under `docs/design/<feature>/` with four numbered docs (overview, design, vision, decisions) plus `specs/` and `references/` subfolders. [CONVENTIONS.md](../CONVENTIONS.md) is the rulebook. The `orbit design check` CLI and the `orbit.design.{init,list,show,check}` MCP tools make scaffolding and decay-detection cheap so agents apply the discipline by default rather than skip it.

This document is the entry point. [2_design.md](./2_design.md) specifies layout, frontmatter, ADR rules, decay-check semantics, and the tool surface. [3_vision.md](./3_vision.md) names open questions (lint enforcement, ADR migration to a queryable artifact, semantic search). [4_decisions.md](./4_decisions.md) records the load-bearing decisions, including the [ORB-00019] promotion of the decay checker to first-class Rust + MCP.

---

## 1. Motivation

AI coding agents are fast enough that "write a design doc" feels like a tax — easy to skip, easy to under-fill, easy to let drift after the implementation lands. Three concrete failure modes the convention exists to prevent:

1. **Drift between docs and code.** Without an enforcement loop, design docs describe the system as it was thought about, not as it currently is. Six months later the doc is worse than nothing — it actively misleads. The decay check ([2_design.md §5](./2_design.md)) compares each doc's `Last updated:` frontmatter line against `git log` timestamps of referenced source files; `make ci` fails when a doc's referenced code has moved on.

2. **Free-form folders that don't read as one system.** When every author picks their own structure, readers cannot navigate. The 4-doc role split (overview / design / vision / decisions) gives a stable contract: open `1_overview.md` for the elevator pitch, `2_design.md` for the current implementation, `3_vision.md` for what's still open, `4_decisions.md` to find why a particular choice was made. Cross-feature reading stays cheap.

3. **ADR rot.** Architecture Decision Records work only if every entry names a real alternative, a forward constraint, and a non-trivial cost ([CONVENTIONS.md §4](../CONVENTIONS.md)). Without rules, the log fills with trivial "we picked X" entries that bury the load-bearing decisions. The 3-of-3 earning rule and the mandatory `Cost:` line keep the signal-to-noise high.

A fourth failure mode is downstream of the others: **agents will not apply a discipline that is expensive to apply.** When scaffolding a new feature requires opening four sibling folders to remember the file names and section orders, agents either skip docs entirely or produce slop. The `orbit.design.init` MCP tool ([2_design.md §6](./2_design.md)) collapses that to one call — the four files exist with correct frontmatter and the standard section skeleton in under a second. Same logic for `orbit design check`: if the operator can run the decay check locally and via `make ci` without remembering which python script to invoke, drift gets caught at PR time instead of months later.

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

### 2.3 Required frontmatter and the `Last updated:` anchor

```
# <Feature> — <Doc Role>

**Status:** <Draft | Accepted>
**Owner:** <agent identity — `claude`, `codex`, etc.>
**Last updated:** YYYY-MM-DD
```

`Last updated:` is the decay-check anchor. The author updates it manually whenever the doc body changes; the check trusts the field over `git log` of the doc itself because trivial reformat commits should not reset the freshness clock. See [4_decisions.md ADR-002](./4_decisions.md) for why.

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

### 2.5 Decay check

For each markdown file under `docs/design/<feature>/`:

1. Read the `Last updated:` field.
2. Extract every relative reference path (e.g. `../../../crates/orbit-knowledge/src/graph_bench.rs`).
3. For each referenced path that exists, run `git log -1 --format=%cs -- <path>` to get the last commit date.
4. Flag the doc as STALE if any referenced file's last commit date is newer than `Last updated:`.

Optional `--include-missing` also fails when a doc references a path that no longer exists in the working tree. See [2_design.md §5](./2_design.md) for the full algorithm.

### 2.6 Tool surface

CLI: `orbit design check [--warn-only] [--include-missing] [--workspace <path>]`. Wired into `make check-design-docs`, which runs as part of `make ci`.

MCP: `orbit.design.init`, `orbit.design.list`, `orbit.design.show`, `orbit.design.check`. Same JSON in/out as the CLI; lets agents scaffold and inspect design folders without shelling out. See [2_design.md §6](./2_design.md).

The legacy [`scripts/check_design_doc_decay.py`](../../../scripts/check_design_doc_decay.py) is retained as a thin wrapper that shells out to `orbit design check` for downstream tooling that still references the script path; the parsing and traversal moved into Rust. See [4_decisions.md ADR-005](./4_decisions.md).

### 2.7 Convention enforcement

[CONVENTIONS.md](../CONVENTIONS.md) is the rulebook. Today, enforcement is cross-review (when one agent reviews another's design folder, the reviewer treats CONVENTIONS.md as a checklist) plus the mechanical decay check. Two enforcement gaps remain open: per-doc lint of required frontmatter + Task References section, and per-ADR lint of the Cost line. See [3_vision.md §1](./3_vision.md).

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Convention rulebook (folder layout, frontmatter, sections, ADR template, glossary) | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| Decay-check core (parse `Last updated:`, walk references, run `git log`) | [crates/orbit-core/src/command/design.rs](../../../crates/orbit-core/src/command/design.rs) | [ORB-00019] |
| `orbit design check` CLI | [crates/orbit-cli/src/command/design.rs](../../../crates/orbit-cli/src/command/design.rs) | [ORB-00019] |
| `orbit.design.{init,list,show,check}` MCP tool registry | [crates/orbit-tools/src/builtin/orbit/design/](../../../crates/orbit-tools/src/builtin/orbit/design/) | [ORB-00019] |
| MCP dispatch into core | [crates/orbit-core/src/runtime/orbit_tool_host/design_tools.rs](../../../crates/orbit-core/src/runtime/orbit_tool_host/design_tools.rs) | [ORB-00019] |
| Make target wiring + CI | [Makefile](../../../Makefile) | [ORB-00019] |
| Legacy python wrapper | [scripts/check_design_doc_decay.py](../../../scripts/check_design_doc_decay.py) | [ORB-00019] |
| Existing feature folders (12) | [docs/design/](../) | [ORB-00006] |
| Adjacent ADR-artifact proposal (would replace `4_decisions.md` with a queryable store) | [docs/design/adr-artifact/](../adr-artifact/) | — |
| Glossary | [references/glossary.md](./references/glossary.md) | — |

---

## Task References

- [ORB-00006] — Refreshed ARCHITECTURE.md and the existing design folders to a consistent layout that became the basis for CONVENTIONS.md.
- [ORB-00019] — Promoted the python decay checker to first-class `orbit design` CLI + `orbit.design.*` MCP tools and rewrote `make check-design-docs` and `scripts/check_design_doc_decay.py` to delegate.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
