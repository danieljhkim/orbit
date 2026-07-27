# Glossary: Project Learnings

Project-specific vocabulary used in [1_overview.md](../1_overview.md), [2_design.md](../2_design.md), [3_vision.md](../3_vision.md), and [4_decisions.md](../4_decisions.md). Standard industry terms (glob, YAML, SQLite, MCP) are excluded unless this feature gives them a specific meaning.

| Term | Meaning |
|------|---------|
| **Active** | Learning lifecycle status indicating the record is eligible for discovery. Opposite: `superseded`. See [2_design.md §7.2](../2_design.md). |
| **Body** | The multi-line markdown content of a learning record — the rule, the reason, and the application guidance. Loaded on demand via `orbit.learning.show`, never injected directly. See [2_design.md §2.1](../2_design.md). |
| **Evidence** | Provenance attached to a learning record — commit SHAs, task IDs, or external refs that produced or substantiate the learning. See [2_design.md §2.1](../2_design.md). |
| **Frozen injection data** | Historical `learning_injected` audit events from automatic delivery, which ended on 2026-07-20. The counters remain readable only as calibration data. See [2_design.md §4.3](../2_design.md). |
| **Learning record** | The first-class Orbit resource representing one piece of project knowledge. YAML on disk, indexed in SQLite, mutated through `orbit.learning.*` tools. See [2_design.md §2](../2_design.md). |
| **Pull surface** | `orbit.search` with `kind: "learning"`, `orbit.learning.show`, and the corresponding CLI commands. Agents use it to discover and retrieve records explicitly. See [2_design.md §4.1](../2_design.md). |
| **Reference comment** | A concise source or workflow comment that identifies a learning or ADR and why it applies. It is a locator, not a copy of the artifact body. See [2_design.md §4.2](../2_design.md). |
| **Scope** | The trigger condition for a learning. Phase 1: path globs and tags evaluated as logical OR. Phase 2: adds symbol IDs and semantic seeds. See [2_design.md §3](../2_design.md). |
| **Semantic seed** | Reserved field (`scope.semantic_seed`) for phase 2. Short text describing what a learning is "about"; used as the embedding source for semantic-similarity ranking. See [3_vision.md §1.2](../3_vision.md). |
| **Stale** | A learning whose referenced files, commits, or tasks no longer exist. Detected opportunistically via `orbit learning prune --stale-only`. See [2_design.md §7.3](../2_design.md). |
| **Summary** | The one-line rule of thumb for a learning, displayed in search results and never substituted with the body. See [2_design.md §2.1](../2_design.md). |
| **Supersede** | Lifecycle transition where a newer learning replaces an older one. Both records persist; the old one's status flips to `superseded` and gains a `superseded_by` back-reference. See [2_design.md §7.2](../2_design.md). |
| **Symbol-aware scope** | Reserved field (`scope.symbols`) for phase 2. Matches against knowledge-graph symbol IDs rather than file paths, surviving renames. See [3_vision.md §1.1](../3_vision.md). |
| **Tag** | Free-form string label on a learning record. Survives file renames where path globs don't. Matched as exact strings in phase 1. See [2_design.md §3.2](../2_design.md). |
