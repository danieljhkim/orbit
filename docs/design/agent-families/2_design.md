---
summary: "Agent Families — Design"
type: design
title: "Agent Families — Design"
owner: human
last_updated: 2026-08-09
last_validated: 2026-07-27
status: Draft
feature: agent-families
doc_role: design
tags: ["agent-families"]
---

# Agent Families — Design

This document describes the current implementation of Orbit agent families and crew-based model assignment. It covers the family registry, workspace config surfaces, task and CLI override surfaces, and where resolved run metadata is persisted.

## 1. Family Registry

The family registry spans `crates/orbit-common/src/types/actor.rs` and `crates/orbit-common/src/types/agent_pair.rs`. `all_agent_families()` returns the supported family identifiers; `agent_from_model()` and `provider_from_model()` infer family and provider from model strings in `actor.rs`; and `infer_agent_family_from_model()` in `agent_pair.rs` remains the conservative recovery helper for older persisted artifacts.

Adding a family is still a cross-cutting change: executor assets, sandbox behavior, provider inference, review automation, and scoreboard code all need review. The fixed registry forces that audit instead of silently accepting unknown families.


## 2. Crew Registry

Workspace config defines one concrete assignment under each `[crews.<name>]`: flat `model`, `provider`, and `backend` fields. Activity roles remain labels, but all resolve through the same assignment.

`crates/orbit-core/src/config/raw.rs` owns the TOML shape, and `crates/orbit-core/src/config/runtime.rs` materializes it into `Crew` values from `orbit-common`. Runtime loading rejects incomplete crews, retired `planner`/`implementer`/`reviewer` role sub-tables with guidance to write flat `model`, `provider`, and `backend` fields, and `[workflow].default_crew` values that do not name a defined crew.

The built-in runtime registry uses model-specific standard crews: Claude provides `opus`, `sonnet`, and `fable`; Codex provides `sol`, `terra`, and `luna`; Gemini provides `gemini`; and Grok provides `grok`. Fresh `orbit init` config filters that registry by detected provider CLIs, always writes `backend = "cli"`, and chooses the first emitted standard crew as `[workflow].default_crew` (`opus`, `sol`, `gemini`, or `grok`). It adds `qa` on Terra when Codex is available, otherwise on Sonnet when Claude is available. With no supported provider CLI, initialization emits neither crews nor a dangling default.

## 3. Task and Tool Surface

`Task` has an optional `crew` field. `orbit.task.add` and `orbit.task.update` validate authored crew names against the current workspace registry, and `orbit.task.start` accepts a one-run `crew` override. The runtime re-validates at start time because the config registry can change between task creation and execution.

The precedence chain is:

1. CLI/tool start override `crew`
2. `Task.crew`
3. `[workflow].default_crew`

`orbit.task.show` surfaces the task field and, when the current registry resolves it, the effective crew name plus one `crew_model` string.

## 4. Run Records

Run-start code resolves the crew before dispatch, emits structured tracing fields for `resolved_crew` and `crew_model`, and persists those strings on the job run record. Persisting resolved values protects audit trails from later config edits.

Legacy records without crew fields still deserialize because the run-record fields are optional. Display code may use `infer_agent_family_from_model()` only as a recovery path for older artifacts.

## 5. Concerns & Honest Limitations

Crew names are workspace-local strings. Renaming or deleting a crew can break a task that still references the old name, though existing run records keep the resolved model strings.

Task-level per-role overrides were deferred; today a task picks an entire crew, not a single replacement planner or reviewer.

## Task References

- ORB-00042: Onboard Grok (xAI) as a first-class supported agent family.
- ORB-00058: Introduce per-task crew override for agent model selection.
- ORB-10315: Seed model-specific crews only for providers available during initialization.
- ORB-10620: Reject retired crew role sub-tables during config load.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
