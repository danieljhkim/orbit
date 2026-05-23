# Glossary: Project Learnings

Project-specific vocabulary used in [1_overview.md](../1_overview.md), [2_design.md](../2_design.md), [3_vision.md](../3_vision.md), and [4_decisions.md](../4_decisions.md). Standard industry terms (glob, YAML, SQLite, MCP) are excluded unless this feature gives them a specific meaning.

| Term | Meaning |
|------|---------|
| **Active** | Learning lifecycle status indicating the record is eligible for injection. Opposite: `superseded`. See [2_design.md §7.2](../2_design.md). |
| **Body** | The multi-line markdown content of a learning record — the rule, the reason, and the application guidance. Loaded on demand via `orbit.learning.show`, never injected directly. See [2_design.md §2.1](../2_design.md). |
| **Dedup set** | Per-session, agent-local set of injected learning IDs. Each push layer consults it before emitting a `<system-reminder>` to prevent duplicate injections across layers. See [2_design.md §4.4](../2_design.md). |
| **Engine pre-prompt injection** | Layer 1 of the push pipeline. `orbit-engine` queries matching learnings before spawning an agent and prepends summaries to the agent prompt. Universal across agents. See [2_design.md §4.1](../2_design.md). |
| **Evidence** | Provenance attached to a learning record — commit SHAs, task IDs, or external refs that produced or substantiate the learning. See [2_design.md §2.1](../2_design.md). |
| **Learning record** | The first-class Orbit resource representing one piece of project knowledge. YAML on disk, indexed in SQLite, mutated through `orbit.learning.*` tools. See [2_design.md §2](../2_design.md). |
| **MCP-sidecar injection** | Layer 2 of the push pipeline. `orbit-mcp` attaches a `learnings` field to MCP tool responses whose arguments or output reference file paths. Cross-agent. See [2_design.md §4.2](../2_design.md). |
| **Pre-tool-use hook** | Layer 3 of the push pipeline. A Claude Code `PreToolUse` hook on `Edit | Write | Read` injects matching learnings before the tool fires. Claude-Code-only. See [2_design.md §4.3](../2_design.md). |
| **Pull surface** | The `orbit.search` tool (with `kind: "learning"`) and the `orbit-learnings` skill. Used for active query at task start or when an agent has time to ask. Complement to push, not substitute. See [2_design.md §6](../2_design.md). |
| **Push layer** | One of the three injection sites that surface learnings into agent context without an agent query. The three layers are engine pre-prompt, MCP-sidecar, and Claude Code hook. See [2_design.md §4](../2_design.md). |
| **Scope** | The trigger condition for a learning. Phase 1: path globs and tags evaluated as logical OR. Phase 2: adds symbol IDs and semantic seeds. See [2_design.md §3](../2_design.md). |
| **Semantic seed** | Reserved field (`scope.semantic_seed`) for phase 2. Short text describing what a learning is "about"; used as the embedding source for semantic-similarity ranking. See [3_vision.md §1.2](../3_vision.md). |
| **Stale** | A learning whose referenced files, commits, or tasks no longer exist. Detected opportunistically via `orbit learning prune --stale-only`. See [2_design.md §7.3](../2_design.md). |
| **Summary** | The one-line rule of thumb for a learning, designed to fit in a `<system-reminder>`. Always injected; never substituted with the body. See [2_design.md §2.1](../2_design.md), [§4.5](../2_design.md). |
| **Supersede** | Lifecycle transition where a newer learning replaces an older one. Both records persist; the old one's status flips to `superseded` and gains a `superseded_by` back-reference. See [2_design.md §7.2](../2_design.md). |
| **Symbol-aware scope** | Reserved field (`scope.symbols`) for phase 2. Matches against knowledge-graph symbol IDs rather than file paths, surviving renames. See [3_vision.md §1.1](../3_vision.md). |
| **Tag** | Free-form string label on a learning record. Survives file renames where path globs don't. Matched as exact strings in phase 1. See [2_design.md §3.2](../2_design.md). |
