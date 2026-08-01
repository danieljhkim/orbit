---
type: design
summary: "Spec: Backend Resolution and Session Constraints"
tags: ["activity-job"]
last_validated: 2026-08-01
---

# Spec: Backend Resolution and Session Constraints

> **Current release scope.** `backend: cli` remains the default and is available for the CLI-backed providers. `backend: http` is also a supported path for providers with a wired HTTP transport (currently Claude); the HTTP-only `loop`/`session:` constraint remains load-bearing.

Activity / Job assets must never reach dispatch with unresolved backend intent. Orbit resolves `backend: auto` to a concrete backend once per run, then validates the concrete shape before execution begins. This keeps backend choice, provider wiring, and session semantics auditable instead of implicit.

## Why This Exists

Backend choice affects runtime behavior in ways that are not interchangeable:

- HTTP vs CLI has different tool-enforcement semantics.
- Only some providers have HTTP transports wired.
- Cross-iteration `session:` binding only makes sense on the HTTP loop path.

Treating backend resolution as an early normalization step keeps those differences explicit.

## Resolution Order

Orbit resolves `backend: auto` using this precedence order:

1. CLI flag override
2. `ORBIT_BACKEND`
3. config `[runtime] backend`
4. hard-coded fallback `cli`

If any tier literally says `auto`, Orbit folds it to the hard-coded fallback. Downstream code must only observe `http` or `cli`.

## Invariants

- `Backend::Auto` does not survive past the orbit-core load path.
- `target: activity:<name>` resolution happens before job execution begins.
- A loop-body step with `session:` must resolve to `backend: http`.
- `backend: http` against an unwired provider fails with structured input validation; it does not silently fall back to CLI.

## Failure Modes

- Invalid backend flag or config values reject the run before dispatch.
- Unresolved `TargetRef` reaching the executor is a caller bug and surfaces as `JobValidation`.
- Loop/session/backend incompatibility rejects the job at load time when possible and again at runtime if a flat target shape still violates the rule.
- `backend: http` plus an unwired provider returns a structured validation error naming the provider and missing HTTP transport.

## Migration Notes

- `schemaVersion: 1` assets are retired and do not participate in this contract.
- `backend: cli` intentionally retains the older CLI-provider runtime implementation; this is not a temporary compatibility shim.
- Jobs that need persistent conversation history across iterations must choose HTTP-capable activities explicitly.

## Agent Signature

Last revised by `codex`.
