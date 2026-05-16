# Agent Families — Glossary

**agent family** — A stable identifier (`claude`, `codex`, `gemini`, `grok`) representing a coherent set of models, CLIs, and integration requirements that Orbit treats uniformly for attribution, execution, and analytics.

**model pair** — The `(orchestrator, helper)` model duo resolved for a family via `resolve_agent_model_pair()`. Used by activity jobs and planning duels.

**duel candidate** — A family in the active `[duel].candidates` allowlist. Defaults to `all_agent_families()` and must keep at least three distinct families so role permutations remain valid.

**all_agent_families()** — The single source of truth function in `orbit-common` that returns the fixed-size array of supported families. Changing its size is intentionally high-friction.

**executor** — The YAML definition (`crates/orbit-core/assets/executors/<family>.yaml`) that describes how `backend: cli` invokes an agent's CLI.
