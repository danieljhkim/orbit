## Context
The agent-loop path is where activity/job can most easily leak provider implementation details, mutable sessions, or role configuration across crate boundaries. The split ADRs all defended the same shape: shared types live low, orbit-core hosts primitive services, the engine dispatches concrete activity specs, and provider/backends remain explicit choices.

## Decision
Keep activity/job types in `orbit-common`, keep orbit-core free of `orbit-agent` transport types, and route `backend: cli` through retained provider runtimes behind a host-resolved executor contract. Scope stateful agent features narrowly: loop `session:` is HTTP-only, Groundhog is its own activity kind, role config from `[agent.<role>]` overrides inline settings field-by-field, task-aware CLI envelopes carry durable run context, and provider static-arg fixups run before sandbox dispatch.

Folded instances:

| ADR | Instance folded into this rollup |
|-----|----------------------------------|
| ADR-005 | Cross-iteration `session:` binding is loop-scoped and HTTP-only. |
| ADR-006 | Retained CLI runtimes implement `backend: cli`. |
| ADR-009 | Groundhog is a sibling activity kind, not an `agent_loop` mode bit. |
| ADR-015 | CLI backend resolves executor args, not just provider commands. |
| ADR-025 | Codex CLI dynamic flags stay in provider runtime config. |
| ADR-027 | `orbit init` writes per-role agent settings. |
| ADR-031 | `[agent.<role>]` config overrides inline `agent_loop` settings at dispatch. |
| ADR-032 | CLI agent envelopes carry durable task and run context. |
| ADR-040 | Provider static-arg fixups apply before sandbox dispatch. |
| ADR-041 | `orbit init` uses a recommendation-first setup wizard. |

## Consequences
- Parsing, validation, dispatch, and CLI display share one Rust type family without making orbit-core depend on provider transport objects.
- CLI and HTTP agent-loop paths remain intentionally different where their capabilities differ, especially around sessions and tool enforcement.
- First-run and per-role agent choices live in user config while YAML stays reusable across workspaces.
- Costs retained from folded entries:
- Cost: `orbit-common` now owns a wider slice of runtime vocabulary and has to stay disciplined about not accreting behavior.
- Cost: session reuse becomes a narrowly scoped feature instead of a general-purpose memory layer.
- Cost: the feature now has materially different semantics between HTTP and CLI, especially around tool enforcement.
- Cost: ActivityV2 gains another sibling variant and the feature family becomes slightly broader.
- Cost: the engine/core boundary is slightly wider than a single string and every smoke host implementing `V2RuntimeHost` must model executor args explicitly.
- Cost: the v2 host boundary exposes a provider-config map, so backend CLI dispatch remains aware of provider-specific runtime settings.
- Cost: until [T20260428-12] landed, the values written to `config.toml` were inert — they round-tripped but did not influence dispatch, so reviewers had to treat the behavior as half-shipped during that window.
- Cost: dispatch now has one more clone-and-mutate path per role-tagged step. The same role might get queried multiple times within one job run; if that ever shows up in profiles, memoize at the executor level rather than in the host trait.
- Cost: the `V2RuntimeHost` seam now has a method that is purely a config-config concern. Tests that build their own mock host get a free `None` default, but a host that wants to exercise the override path has to opt in explicitly.
- Cost: CLI stdin blobs now contain more task prose, so audit blob readers should continue treating those blobs as diagnostic artifacts rather than small control messages.
- Cost: provider static-arg fixups mean executor YAML values such as Claude's `--debug-file` path are no longer honored verbatim; maintainers must read dispatcher behavior alongside assets.
- Cost: prompt collection now owns display formatting and a small choice loop, so tests must cover interaction flow in addition to config values.
