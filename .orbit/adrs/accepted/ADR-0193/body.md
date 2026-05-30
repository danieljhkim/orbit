## Context
`orbit init` needs a config that reflects installed agent surfaces, but live detection reads ambient PATH and API-key environment. Re-running detection during `RuntimeConfig::load_layered` would make crew and duel resolution vary between invocations without a config diff.

## Decision
Agent availability is detected once during init using `DetectedAgents`, rendered into `config.toml`, and then treated as static configuration. Runtime config loading continues to use file contents and built-in fallbacks only; it never probes PATH or environment.

## Consequences
- Fresh configs pick a sensible `default_crew` and duel candidate set for the host that created them.
- Runtime behavior is deterministic for a given config file, including hot config loads.
- Cost: A user who installs or removes agent CLIs after init must edit or regenerate config instead of getting automatic runtime drift.