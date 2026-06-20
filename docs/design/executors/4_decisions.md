---
title: Executors — Decisions
owner: claude
last_updated: 2026-06-20
status: Draft
feature: executors
doc_role: decisions
type: design
summary: ADR log for executor registration and the External Executor Protocol.
tags: [executors]
paths: ["crates/orbit-engine/src/executor/**", "crates/orbit-common/src/types/executor_def.rs"]
related_features: [executors]
related_artifacts: [ORB-00384, ORB-00400, ADR-0196]
---

# Executors — Decisions

This is the append-only ADR log for the `executors` feature. Entries are ordered
by ascending global ADR number. Each entry is the long-form narrative keyed on a
global ID allocated through `orbit.adr.add`; the ADR store is the source of truth
for status, owner, `related_features`, and `related_tasks`. Resolve any global ID
with `orbit tool run orbit.adr.show --input '{"id":"ADR-0196"}'`.

Layout note: as of [ORB-00400], this folder is intentionally decisions+specs-only.
[ADR-0196] and [specs/external-executor-protocol.md](./specs/external-executor-protocol.md)
are the load-bearing docs for the shipped External Executor Protocol; placeholder
`1_overview.md`, `2_design.md`, and `3_vision.md` docs would imply a broader
executor feature narrative that this work has not established. Add numbered docs
only when a future executor-architecture task owns that narrative, and retire
this exception in the same PR.

---

## ADR-0196 — External Executor Protocol for dynamic out-of-process executor registration

**Status:** Accepted · 2026-06 · [ORB-00384] (Tier 1: defined the protocol, added the `external` executor type, shipped a conformance test)

**Context.** Orbit's `ExecutorType` is a sealed enum and `load_from_defs` is a closed `match`, so a homegrown executor can only be added by forking orbit-engine — an `internal`-tier crate with no downstream guarantees. Yet `DirectAgentExecutor` already implements an out-of-process transport (spawn `command`, write a request envelope to stdin, map the process exit code / stderr to an outcome): the capability exists but is undocumented and coupled to the agent-family `direct_agent` path.

**Decision.** Promote that transport into a documented, versioned **External Executor Protocol v1** and expose it through a new `ExecutorType::External` (wire value `external`). A homegrown executor is registered by dropping a YAML executor def that points at a binary/script speaking the protocol — no recompile, no linking, language-agnostic. In-process Rust extension (an `ExecutorFactory` registry plus a runtime injection seam) is explicitly deferred to a separate Tier 2 decision.

**Consequences.**
- Most homegrown executors become config-only: a YAML def plus a conforming binary, with zero changes to Orbit.
- The stdin request envelope (`schemaVersion: 1`) and the exit-code result semantics become a stability commitment — once v1 ships, the request/result shape is a contract that must be versioned, not changed in place. stdout is captured as audit data but is not parsed into workflow state in v1 (a structured stdout result envelope is a reserved, additive extension point).
- **Sandbox finding (during execution).** The task premise that `direct_agent` routes `FsProfile`→sandbox was inaccurate: the registry-path transport `DirectAgentExecutor` uses (and which `external` now shares) runs `NoSandbox`, and the registry `ExecutionContext` carries no `FsProfile`. Real `FsProfile`→OS-sandbox enforcement lives only in the separate V2 `activity_job` path. Tier 1 therefore ships **exact parity**: `external` and `direct_agent` produce a byte-identical `ExecRequest` and both run unsandboxed; the def's `sandbox`/`allow_fallback` fields are inert for `external`. This does not widen the sandbox-bypass surface relative to `direct_agent`, but it adds no OS sandboxing either — registering an external executor is arbitrary code execution with the runner's privileges. Real `FsProfile`→OS sandbox for `external` is deferred to Tier 2 (needs the V2 context). See [ORB-00384] comments.
- Executors needing a non-subprocess transport (in-process SDK, gRPC, internal queue) are NOT served by Tier 1 and must wait for Tier 2.
- Cost: a documented wire protocol is a long-lived backward-compatibility obligation — every future executor capability must be expressible as an additive, versioned envelope field, and a conformance harness must be maintained so adopters do not silently depend on undocumented behavior.

---

## Task References

- **[ORB-00384]** — External Executor Protocol v1: define the contract, add `ExecutorType::External`, register a generic external-process executor, document the spec, ship a conformance test.
- **[ORB-00400]** — recorded the `executors` folder as a decisions+specs-only layout exception while refreshing design-doc ownership conventions.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
