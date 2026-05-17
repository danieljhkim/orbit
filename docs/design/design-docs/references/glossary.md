# Glossary: Design Docs

Terms specific to the design-docs convention and the `orbit design` tooling. Standard markdown / git terminology (frontmatter, commit, link) is excluded unless the convention gives the term a specific meaning.

| Term | Meaning |
|------|---------|
| **At a Glance table** | The required §3 of `1_overview.md` mapping each concern to the file that owns it and the task that shipped the work. See [2_design.md §3](../2_design.md). |
| **Cost line** | A bullet in an ADR's `Consequences` section labeled `Cost: …`. Mandatory per [CONVENTIONS.md §4](../../CONVENTIONS.md); enforced by review. See [2_design.md §4](../2_design.md). |
| **Decay** | The state of a design doc whose `Last updated:` field is older than the most recent commit on a referenced source file. Detected by `orbit design check`. See [2_design.md §5](../2_design.md). |
| **Earning rule (3-of-3)** | The criterion a decision must satisfy to become an ADR rather than design-doc prose: real alternative, forward constraint, non-trivial cost. See [2_design.md §4](../2_design.md). |
| **Feature folder** | A subdirectory of `docs/design/` containing one feature's four numbered docs plus `specs/` and `references/` subfolders. Lowercase, hyphenated, singular. See [2_design.md §1](../2_design.md). |
| **Format explainer** | The opening paragraph of `4_decisions.md` describing the ADR template and listing exceptions (e.g. retroactive entries without task IDs). |
| **Last updated anchor** | The `**Last updated:** YYYY-MM-DD` line in each numbered doc's frontmatter; the freshness reference for the decay check. See [4_decisions.md ADR-002](../4_decisions.md). |
| **Numbered doc** | One of the four required files in a feature folder: `1_overview.md`, `2_design.md`, `3_vision.md`, `4_decisions.md`. Each has a fixed role and required section list. See [2_design.md §3](../2_design.md). |
| **Owner** | The accountable agent family in a doc's frontmatter (one of `codex`, `claude`, `gemini`, or `grok`); not a committer list or full model string. See [2_design.md §2](../2_design.md). |
| **Rollup ADR** | A consolidated ADR that absorbs a cluster of accepted ADRs all instantiating the same underlying decision. Folded entries keep their numbers with `Status: Superseded by ADR-NNN (folded)`. See [CONVENTIONS.md §4a](../../CONVENTIONS.md). |
| **Scope paragraph** | The opening paragraph of `2_design.md` and `3_vision.md`, naming what the doc covers and what it deliberately does not. |
| **Stale finding** | A `DesignDecayFinding` record produced by the decay check: the doc path, its `Last updated:`, and the list of referenced files with newer commit dates. See [2_design.md §5](../2_design.md). |
| **Task References section** | The mandatory closing section of every numbered doc, listing only the task IDs cited in that doc. See [CONVENTIONS.md §3](../../CONVENTIONS.md). |
| **`_archive/`** | Reserved subfolder under `docs/design/` for retired feature folders. Skipped by `orbit.design.list` and the decay check. See [CONVENTIONS.md §7](../../CONVENTIONS.md). |
