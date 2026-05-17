# Agent Families — Overview

**Status:** Draft
**Owner:** grok
**Last updated:** 2026-05-16

Orbit models AI coding systems as first-class agent families and now groups concrete role assignments into named crews. Families answer "which provider/tooling is this model from?"; crews answer "which planner, implementer, and reviewer lineup should run this task?"

## 1. Motivation

Orbit was initially built around three dominant agent CLIs: Claude Code, Codex (OpenAI), and Gemini. As more agents gained strong coding capabilities, including Grok via Grok Build and the xAI API, Orbit needed stable family identifiers for attribution, execution, review, and analytics.

Family defaults were originally also used to pick activity role models, but that made per-task model experiments require editing workspace config. The crew registry introduced by [ORB-00058] separates model selection from family discovery: workspaces define named lineups in `[crews.<name>]`, tasks may pin `crew`, and operators may override the crew for a single run. Duel-plan role selection has a narrower workspace knob under `[duel]`: operators can allowlist candidate families and pin duel-only orchestrator models without changing executor YAML or affecting non-duel callers.

## 2. Core Concepts

- **Agent Family:** A stable identifier such as `claude`, `codex`, `gemini`, or `grok` used for routing, attribution, and legacy model inference.
- **Model Inference:** `agent_from_model()` and `infer_agent_family_from_model()` map concrete model strings to families. `infer_agent_family_from_model()` remains for legacy artifact recovery.
- **Crew:** A named planner, implementer, and reviewer assignment loaded from `.orbit/config.toml` under `[crews.<name>]`.
- **Default Crew:** `[workflow].default_crew` names the workspace fallback when a task does not specify `crew`.
- **Executor:** A YAML definition in `crates/orbit-core/assets/executors/<family>.yaml` describing how to invoke an agent CLI.
- **Sandbox Surface:** Provider-specific state directories and lockfile rules required for safe `macos-sandbox-exec` execution.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Family inference and crew resolver | `crates/orbit-common/src/types/agent_pair.rs` | [ORB-00042], [ORB-00058] |
| Task-level crew field | `crates/orbit-common/src/types/task.rs` | [ORB-00058] |
| Runtime config loading | `crates/orbit-core/src/config/runtime.rs` | [ORB-00058] |
| Default config template | `crates/orbit-core/assets/config/default-config.toml` | [ORB-00058] |
| Run-time crew resolution | `crates/orbit-core/src/runtime/engine/crew.rs` | [ORB-00058] |
| Duel candidate and model config | `crates/orbit-core/src/config/runtime.rs` | [ORB-00072] |
| Tool and CLI crew surfaces | `crates/orbit-tools/src/builtin/orbit/task/` | [ORB-00058] |
| Grok family onboarding | `crates/orbit-common/src/types/agent_pair.rs` | [ORB-00042] |

## Task References

- ORB-00042: Onboard Grok (xAI) as a first-class supported agent family.
- ORB-00043: Add Grok to agent_from_model, all_agent_families, and provider_from_model.
- ORB-00048: Harden duels, scoreboards, review sync, friction stats, and analytics for the fourth family.
- ORB-00058: Introduce per-task crew override for agent model selection.
- ORB-00072: Make duel-plan agent pool and per-family model configurable via `[duel]`.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
