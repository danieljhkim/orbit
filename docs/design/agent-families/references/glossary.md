---
type: design
summary: "Agent Families — Glossary"
tags: ["agent-families"]
last_validated: 2026-08-01
---

# Agent Families — Glossary

**agent family** — A stable identifier (`claude`, `codex`, `gemini`, `grok`) representing a coherent set of models, CLIs, and integration requirements that Orbit treats uniformly for attribution, execution, and analytics.

**model pair** — The `(orchestrator, helper)` model duo represented by `AgentModelPair`, loaded from an executor's `model_pair_override` via `configured_agent_model_pair()` and exposed to planning duels through `resolved_agent_model_pair()`.

**duel candidate** — A family in the active `[duel].candidates` allowlist. Defaults to `all_agent_families()` and must keep at least three distinct families so role permutations remain valid.

**all_agent_families()** — The single source of truth function in `orbit-common` that returns the fixed-size array of supported families. Changing its size is intentionally high-friction.

**executor** — The YAML definition (`crates/orbit-core/assets/executors/<family>.yaml`) that describes how `backend: cli` invokes an agent's CLI.
