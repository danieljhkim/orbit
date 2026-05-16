# Agent Families — Overview

**Status:** Draft
**Owner:** grok
**Last updated:** 2026-05-16

Orbit models certain AI coding agents as first-class "agent families". A family represents a coherent set of models, CLIs, configuration locations, sandbox requirements, and integration surfaces that Orbit treats with consistent behavior for attribution, execution, review, and analytics.

## 1. Motivation

Orbit was initially built around three dominant agent CLIs: Claude Code, Codex (OpenAI), and Gemini. As more agents gained strong coding capabilities (including Grok via Grok Build and the xAI API), it became necessary to treat additional agents as peers rather than second-class or unknown entities.

Treating an agent as a peer family ensures:
- Correct provenance on tasks, review threads, commits, and friction reports
- Participation in cross-agent workflows (planning duels, automated review)
- Safe execution under `backend: cli` via dedicated executors and macOS sandbox profiles
- First-class MCP and onboarding support via `orbit mcp init`
- Model-pair resolution for orchestrator/helper roles in activities

## 2. Core Concepts

- **Agent Family**: A stable identifier (e.g. `claude`, `codex`, `gemini`, `grok`) used throughout the system for routing, attribution, and configuration.
- **Model Inference**: `agent_from_model()` and `infer_agent_family_from_model()` map concrete model strings (e.g. `claude-opus-4-7`, `grok-4`, `gemini-3.1-pro-preview`) to families.
- **Model Pair**: Each family has a default `(orchestrator, helper)` pair used by activity jobs and duel planning (see `resolve_agent_model_pair`).
- **Executor**: A YAML definition in `crates/orbit-core/assets/executors/<family>.yaml` describing how to invoke the agent's CLI.
- **Sandbox Surface**: Provider-specific state directories and lockfile rules required for safe `macos-sandbox-exec` execution.

## 3. Current Families (as of 2026-05)

| Family  | Primary Models                  | Provider | Notes |
|---------|----------------------------------|----------|-------|
| claude  | claude-*, opus-*, sonnet-*      | anthropic | Claude Code |
| codex   | gpt-*, o1-*, o3-*               | openai    | Codex / OpenAI CLIs |
| gemini  | gemini-*                        | google    | Gemini CLI |
| grok    | grok-*, grok3*                  | xai       | Grok Build + xAI API (added ORB-00042) |

The authoritative list lives in `all_agent_families()` in `crates/orbit-common/src/types/agent_pair.rs`. This is intentionally a fixed-size array so that adding a family forces review of all call sites.

## Task References

- ORB-00042: Onboard Grok (xAI) as a first-class supported agent family (epic)
- ORB-00043: Add Grok to agent_from_model, all_agent_families, and provider_from_model
- ORB-00048: Harden duels, scoreboards, review sync, friction stats, and analytics for the fourth family
- ADR-0151: Add Grok (xAI) as a fourth peer agent family

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
