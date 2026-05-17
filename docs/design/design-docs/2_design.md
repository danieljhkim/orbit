# Design Docs — Design

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-17

This document specifies the design-docs convention as currently shipped: the per-feature folder layout, the required frontmatter and sections per numbered doc, the ADR template and earning rules, the decay-check algorithm, and the CLI + MCP tool surface introduced by [ORB-00019]. The convention itself is in [CONVENTIONS.md](../CONVENTIONS.md); this design doc explains the *implementation* of the convention — what lives where in the Rust crates and how the pieces fit together.

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

Folder names are lowercase, hyphenated, and singular (`knowledge-graph`, not `knowledge-graphs`). Folders prefixed with `_` (e.g. `_archive/`) are ignored by `orbit.design.list` and the decay check — that is the documented retirement path. No top-level narrative files outside the numbered four are permitted; readers should never have to discover a fifth file in the folder. See [CONVENTIONS.md §1](../CONVENTIONS.md), §10, §7.

The four numbered docs and the two subfolders are scaffolded atomically by [`init_feature`](../../../crates/orbit-core/src/command/design.rs) and exposed via the `orbit.design.init` MCP tool ([§6](#6-tool-surface-cli--mcp)).

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

`Owner` is the *accountable* agent family (one of `codex`, `claude`, `gemini`, or `grok`), not a committer list or full model string. `Last updated:` is parsed by the decay check ([§5](#5-decay-check-algorithm)) and is authoritative — see [4_decisions.md ADR-002](./4_decisions.md) for why this overrides `git log` of the doc itself.

The `init_feature` scaffold writes today's date and the caller's identity into all four frontmatter blocks at scaffold time; the `orbit.design.show` and `orbit.design.list` tools surface the parsed values ([§6](#6-tool-surface-cli--mcp)).

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

## 5. Decay Check Algorithm

Implemented in [`check_workspace_path`](../../../crates/orbit-core/src/command/design.rs). For each markdown file under `docs/design/`:

1. **Read the `Last updated:` field.** Frontmatter parsing accepts `**Last updated:** YYYY-MM-DD`. Docs without the field are silently skipped (CONVENTIONS.md is one such — it has `Last updated:` but is not under a feature folder; the walker still picks it up).
2. **Extract relative references.** Regex-scan the doc for inline links matching `(./...)` and `(../...)` patterns; resolve each against the doc's directory. Plain text mentions of file paths are ignored — only markdown links count.
3. **For each referenced path that exists:** run `git log -1 --format=%cs -- <path>` to get the last commit date in `YYYY-MM-DD` form. Paths that do not exist are recorded separately in `missing_references` ([§5.2](#52-include-missing-semantics)).
4. **Compare.** If any referenced file's last commit date is strictly greater than the doc's `Last updated:`, the doc is flagged as `STALE` and each newer reference is included in the finding's `newer_references` list with its commit date.

The output report ([`DesignCheckReport`](../../../crates/orbit-core/src/command/design.rs)) is `{ findings, missing_references, stale_found, missing_found }`. The text formatter renders one `STALE   <doc-path>  (declared YYYY-MM-DD) — newer code:` line per finding, followed by indented `<commit-date>  <path>` lines for each newer reference.

### 5.1 Why `Last updated:` and not `git log` of the doc

A trivial whitespace or wording fix should not reset the freshness clock; otherwise a typo-fix PR makes a six-month-stale doc look fresh. The author's manual update of the field is the assertion "I have read this doc end-to-end and it still describes the system." See [4_decisions.md ADR-002](./4_decisions.md).

### 5.2 `--include-missing` semantics

Default mode flags only stale references. With `--include-missing`, missing references (paths in markdown links that no longer exist in the tree) also cause a non-zero exit. The report always populates `missing_references`; the flag only changes pass/fail. This is intentional for two reasons: missing references are sometimes load-bearing (e.g. a deleted module a vision doc still discusses as the historical reason for the current shape), and a renamed-file PR should not auto-fail every doc that referenced the old path. The PR author opts in by passing `--include-missing` when they want strict link checking.

---

## 6. Tool Surface (CLI + MCP)

### 6.1 CLI: `orbit design`

```
orbit design check [--warn-only] [--include-missing] [--workspace <path>]
```

Defined in [`crates/orbit-cli/src/command/design.rs`](../../../crates/orbit-cli/src/command/design.rs). Default: exit 1 if any doc is STALE; print findings to stdout. Flags:

- `--warn-only` — print findings, exit 0 regardless. Used by editors / pre-commit hooks that want diagnostics without blocking.
- `--include-missing` — also fail when references point to nonexistent files ([§5.2](#52-include-missing-semantics)).
- `--workspace <path>` — override the workspace root (defaults to `cwd`). Used by tests and by callers that drive the check from outside the repo root.

The CLI is the thin shim; all logic lives in `orbit-core::command::design::{check_workspace_path, format_check_report, check_fails}`. `make check-design-docs` invokes `cargo run --quiet -p orbit-cli --bin orbit -- design check`, and the legacy [`scripts/check_design_doc_decay.py`](../../../scripts/check_design_doc_decay.py) was reduced to a thin wrapper around that same invocation.

### 6.2 MCP tools: `orbit.design.{init,list,show,check}`

Tool registry definitions in [`crates/orbit-tools/src/builtin/orbit/design/`](../../../crates/orbit-tools/src/builtin/orbit/design/); dispatch into the core in [`crates/orbit-core/src/runtime/orbit_tool_host/design_tools.rs`](../../../crates/orbit-core/src/runtime/orbit_tool_host/design_tools.rs). All four accept `workspace?: string` (defaults to server cwd) and the standard agent/model identity params; specific shapes:

| Tool | Required input | Returns |
|------|----------------|---------|
| `orbit.design.init` | `feature: string`, optional `owner: string` | `DesignFeatureSummary` for the scaffolded folder |
| `orbit.design.list` | — | `Array<DesignFeatureSummary>` (one per non-`_`-prefixed subfolder) |
| `orbit.design.show` | `feature: string` | `DesignFeatureSummary` for the named feature; typed `NotFoundKind::DesignFeature` error if absent |
| `orbit.design.check` | optional `include_missing: bool` | `{ findings, missing_references, stale_found, missing_found }` matching the CLI report |

`DesignFeatureSummary` is `{ feature, docs: { "1_overview.md": { path, owner, last_updated, decay_status }, ... }, specs_path, references_path }`. `decay_status` is `"fresh" | "stale"` per-doc; the per-doc check runs the same algorithm as the workspace-wide check ([§5](#5-decay-check-algorithm)) but scoped to one file's references.

### 6.3 Init scaffolding behavior

`init_feature` validates the name (lowercase, hyphenated, path-safe — rejects spaces, uppercase, special chars), errors if the folder already exists ([4_decisions.md ADR-006](./4_decisions.md) bans clobber), creates `specs/` and `references/`, and writes the four numbered docs with frontmatter populated from today's date and the caller's identity. The body of each scaffolded doc contains the required section headers as empty placeholders so the author starts from the canonical structure rather than a blank file. CONVENTIONS.md is *not* re-seeded by `init_feature` — it lives once at `docs/design/CONVENTIONS.md` and predates this tooling; `seed_design_conventions` exists for `orbit init` to drop a copy when scaffolding a fresh repo.

---

## 7. Concerns & Honest Limitations

- **Reference extraction is regex-based, not a markdown parser.** The current implementation matches `(./...)` and `(../...)` link patterns; it does not understand reference-style links (`[label][ref]`) or HTML `<a href="...">`. In practice every existing design doc uses inline `(path)` links and this has not caused a missed reference, but a future doc style change could quietly break decay coverage.
- **`Last updated:` is a manual discipline.** The whole decay check is downstream of the author updating one field. If they forget, the doc looks fresh until the next reviewer notices. The trade is intentional ([4_decisions.md ADR-002](./4_decisions.md)) — `git log` on the doc would create false freshness from cosmetic edits — but it does mean the discipline is not machine-enforceable today.
- **Lint coverage is partial.** The decay check catches "doc is stale relative to code" but not "doc is missing required sections," "ADR is missing a Cost line," or "frontmatter is malformed." See [3_vision.md §1.2](./3_vision.md).
- **The 3-of-3 ADR earning rule is judgmental.** Reasonable agents disagree on whether a given decision crosses the bar. There is no automated check; cross-review is the only enforcement, and rejected ADRs occasionally re-litigate the rule itself rather than the substance.
- **No cross-feature decision storage.** A decision that touches three features lives in whichever folder its author opened first. The proposed [adr-artifact](../adr-artifact/) feature would dissolve this by lifting ADRs into a queryable artifact store with N:M `related_features`. Until that ships, cross-feature ADRs are duplicated or silently homed in one folder.
- **The decay check is git-bound.** It uses `git log -1 --format=%cs -- <path>` to date references. Out-of-tree references (e.g. linking to an external repo) are skipped silently. Symlinks and submodules are not traversed.
- **The init scaffold writes the same date into all four docs even when only one will be authored today.** This is harmless — the date is the floor, not the ceiling — but means a freshly scaffolded folder shows four `2026-05-14` dates when only `1_overview.md` may be the one being actively shaped. Authors are expected to bump the date on the docs they actively touch.

---

## Task References

- [ORB-00019] — Promoted the python decay checker to first-class `orbit design` CLI + `orbit.design.*` MCP tools, scaffolded the per-feature folder layout via `orbit.design.init`, and rewrote `make check-design-docs` and `scripts/check_design_doc_decay.py` as thin wrappers.
- [ORB-00090] — Aligned the `Owner` field contract with the agent-family identity convention.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
