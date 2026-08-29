# orbit-engine

Project instructions for the activity/job execution engine.

## One job

Run a v2 activity or job to completion: resolve inputs through templates,
dispatch to a provider CLI or a deterministic action, record step results and
audit rows, and handle retry, resume, fan-out, and concurrency. It sits above
`orbit-agent`, `orbit-exec`, `orbit-store`, and `orbit-tools`, and is consumed
by `orbit-core`.

It owns *how* work runs. It does not own *what* work exists or who may ask for
it: catalog placement, workspace resolution, authorization, and task lifecycle
policy are Core's. The engine must never depend on `orbit-core`, `orbit-cmd`,
or any transport crate.

## `RuntimeHost` is the capability boundary

[`context::hosts`](src/context/hosts.rs) defines the single trait through which
job execution reaches anything the engine does not own — task reads and
updates, event emission, invocation queries, agent dispatch. Core implements it
in its `adapter/engine_host` module.

When execution needs a new capability, **add a method to `RuntimeHost`** with a
default that returns the `unsupported ... capability` error, and implement it in
Core. Do not smuggle the capability in by widening a dependency, threading an
`OrbitRuntime`-shaped handle through, or reading state directly from
`orbit-store` where a host method belongs.

The one deliberate exception to "engine stays provider-agnostic" is
[`activity_job::cli_runner`](src/activity_job/cli_runner), which names
`orbit_agent::{Agent, AgentConfig}` directly. That edge exists so `orbit-core`
stays clean of `orbit-agent` types; keep it inside `cli_runner` rather than
letting agent types spread across the engine.

## Internal layout

- [`activity_job/`](src/activity_job) — the run path: asset loading and
  catalogs, crew resolution, the dispatcher, `cli_runner` (argv, envelope,
  spawn, supervisor, orchestrator), `job_executor` (step, target, loop, fan-out,
  parallel, recovery, templating, validate), and the audit sinks.
- [`executor/automation/`](src/executor/automation) — the deterministic actions
  a job step can invoke without an agent. The `vcs` subtree (commit, push, PR,
  worktree, freshness, handoff) is the largest; it is organized by *operation*,
  and each new operation gets a file, not another arm in an existing one.
- [`context/`](src/context) — split by concern (`hosts`, `outcome`, `env`) and
  re-exported so `crate::context::X` paths stay stable.
- [`template.rs`](src/template.rs), [`condition.rs`](src/condition.rs) — pure
  rendering and predicate evaluation, no I/O.

The run path is where "several phases in one function" pressure shows up first.
The `job_executor` and `cli_runner` splits are the reference for relieving it:
name the phase, give it a file, keep the top-level flow readable.

## Crate-specific invariants

- **Boundary errors are translated here.** `DispatchError` and `CatalogError`
  are registered in `scripts/check-error-translation.sh`; their
  `dispatch_error_to_orbit` / `catalog_error_to_orbit` translators must stay in
  this crate ([`error_translation.md`](../../docs/design-patterns/error_translation.md)).
  A caller crate that maps their variants to `OrbitError` fails CI.
- **Redaction is not local.** `scripts/check-artifact-redaction-guardrail.sh`
  forbids `fn redact_*` in
  [`cli_runner/orchestrator.rs`](src/activity_job/cli_runner/orchestrator.rs)
  and [`cli_runner/argv.rs`](src/activity_job/cli_runner/argv.rs); argv and
  output redaction flows through `orbit_common::security::redaction`.
- **Child environments are composed, never inherited.** Spawn paths build the
  environment with `orbit_common::security::child_env` over a cleared
  environment, plus the shared provenance variables in
  [`context/env.rs`](src/context/env.rs).
- **Resume must stay correct.** Step results are durable; a change to
  `job_executor` step accounting needs coverage in the resume/recovery tests,
  not just the happy path.

## Tests

Sibling `tests/` directories for unit coverage
([`test_layout.md`](../../docs/design-patterns/test_layout.md)); crate-root
[`tests/`](tests) holds the end-to-end v2 runtime, CLI-agent, and
name-resolution integration tests; [`examples/`](examples) holds the runnable
v2 smoke programs. Put a new cross-module behavior in the crate-root
integration tests only when it genuinely exercises the public surface
end-to-end.
