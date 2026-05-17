## Context

Orbit's agent modeling (introduced across activity-job, auditability, and policy-sandbox work) treated Claude, Codex, and Gemini as the complete set of first-class CLI agent families. `all_agent_families()` was deliberately a fixed-size array of 3, `agent_from_model` / `infer_agent_family_from_model` only recognized those three prefixes, executor YAMLs and macOS sandbox profiles existed only for those three, and `orbit mcp init` only knew how to configure those three clients.

Grok Build (and the xAI API surface) is now a real, actively used client in the Orbit development workflow. Treating it as an unknown/foreign agent produces invisible attribution, broken duels/scoreboards, unsafe sandbox execution, and inconsistent onboarding.

Real alternatives considered: (1) treat Grok as a variant of Codex/OpenAI-compat, (2) keep it as an unmodeled third-party agent forever, (3) add it as a true peer family with the same rights and obligations as the original three.

## Decision

We add "grok" as a fourth peer agent family alongside claude/codex/gemini.

This means:
- Extending `agent_from_model`, `infer_agent_family_from_model`, `all_agent_families()`, `resolve_agent_model_pair`, and `provider_from_model` to recognize Grok model strings and map them to a stable family identifier ("grok") and provider ("xai").
- Adding a `grok.yaml` executor definition and the corresponding CLI runner + sandbox support.
- Adding a Grok provider to the `orbit mcp init` machinery so it can generate `.grok/config.toml` entries.
- Updating all documentation, tests, duels, scoreboards, releasing processes, and repo-root configuration directories to treat Grok as a first-class peer.

## Consequences

- Grok-authored tasks, reviews, and commits will be correctly attributed and will participate in planning duels and analytics.
- `backend: cli` execution against Grok (via xAI-compatible wrapper or future official CLI) will be sandbox-safe on macOS.
- `orbit mcp init` will support Grok Build users with the same one-command experience as the other three agents.
- The fixed-size array contract in `all_agent_families()` will now be 4; every call site that assumed "exactly three" must be audited.
- Cost: We accept a permanent increase in the number of agent families we must maintain (executors, sandbox rules, MCP providers, model-pair defaults, docs). Future families will be cheaper to add, but each still carries non-trivial integration cost in sandboxing and client configuration.