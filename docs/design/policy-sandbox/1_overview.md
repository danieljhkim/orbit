# Policy & Sandboxing — Overview

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-16

> **Sandbox backend status.** The only OS-level sandbox backend implemented today is `macos-sandbox-exec`. `ExecutorSandboxKind` (`crates/orbit-common/src/types/executor_def.rs`) defines no Linux or Windows variant, and `EnvironmentHost::resolve_executor_sandbox` (`crates/orbit-core/src/runtime/v2_host/sandbox.rs`) rejects `macos-sandbox-exec` on non-macOS platforms. On Linux and Windows the spawned agent subprocess runs without OS-level isolation; HTTP-tool `fs.*` enforcement and process supervision still apply. A Linux backend is named in [3_vision.md](./3_vision.md) as future work but is not in `2_design.md`'s shipped contract. [T20260505-23]

Policy & Sandboxing is Orbit's safety surface for filesystem access and process execution. It combines v2 `PolicyDef` profiles, global `denyRead` / `denyModify` rules, HTTP-tool fs enforcement, optional macOS `sandbox-exec` wrapping for CLI agents, and `orbit-exec` process supervision. [2_design.md](./2_design.md) documents what ships today; [3_vision.md](./3_vision.md) names the gaps to a fuller isolation contract.

---

## 1. Motivation

Orbit runs agents against user repositories, so the safety boundary is a product feature rather than an internal hygiene concern.

1. **Default paths stay explicit.** Omitting `fsProfile:` maps to `unrestricted`, then still runs through profile resolution and global denies.
2. **Profiles are activity-scoped.** A job can mix profiles by activity; evaluation happens per call, not by mutating a process-global mode.
3. **Deny rules are global.** `denyRead` and `denyModify` are injected into every resolved profile as negated rules, so workspace policy can narrow but not erase global denies.
4. **Execution has two layers.** `orbit-exec` always supervises child processes; CLI-agent filesystem narrowing is OS-enforced only where the configured executor uses macOS `sandbox-exec`.
5. **Denials are evidence.** HTTP fs denials emit through `FsAuditLogger` into `V2AuditEvent` filesystem entries; [auditability](../auditability/) owns durable storage.

---

## 2. Core Concepts

### 2.1 Policy is v2-only

`PolicyDef` accepts only schema v2: `denyRead`, `denyModify`, and named `FsProfile` entries with `read` / `modify` glob rules. Workspace profiles override globals by name; global denies accumulate.

### 2.2 Profile resolution materializes an implicit `unrestricted`

When an activity omits `fsProfile:`, the v2 host uses `UNRESTRICTED_FS_PROFILE`. If the policy does not define that profile, the resolver synthesizes `read: ["./**"]` and `modify: ["./**"]`, then injects global denies.

### 2.3 Path evaluation is last-match-wins over a normalized rule list

`PolicyDef::check_path` evaluates normalized workspace-relative paths against positive and negated rules. The last matching rule wins. Empty positive sets deny with `[]`; unmatched positive sets deny with `<no matching rule>`.

### 2.4 Enforcement depends on backend

HTTP activities enforce policy in the `orbit-tools` `fs.*` builtins before any read or modify. Denials return `OrbitError::PolicyDenied` and emit audit events. CLI activities do not call those builtins; they rely on harness delegation plus the configured executor sandbox, currently macOS `sandbox-exec` for supported CLI agents.

### 2.5 Exec supervision is not default OS isolation

`orbit-exec::run_process` spawns a process-group leader, drains stdout/stderr, installs SIGINT/SIGTERM handlers, and on timeout or signal sends SIGTERM to the group with a 5 second grace before SIGKILL. The default `Sandbox` impl remains `NoSandbox`; OS isolation is added by specific executor wrappers, not the default runner.

---

## 3. At a Glance

| Concern | Where it lives | Primary task ID |
|---------|----------------|-----------------|
| Policy schema and validation | `crates/orbit-common/src/types/policy_def.rs`, `crates/orbit-common/src/types/resource.rs` | [T20260416-0728] |
| Allow/deny enum | `crates/orbit-common/src/types/policy_decision.rs` | [T20260426-0622] |
| Policy facade | `crates/orbit-policy/src/{lib,engine,evaluator,decision}.rs` | [T20260416-0728] |
| Profile resolution + deny injection | `crates/orbit-common/src/types/policy_def.rs` (`effective_profile`, `check_path`) | [T20260416-0728] |
| Implicit `unrestricted` materialization | `crates/orbit-core/src/runtime/v2_host/mod.rs` (`tool_context_for_activity`) | [T20260419-0503] |
| Tool-layer fs enforcement | `crates/orbit-tools/src/builtin/fs/mod.rs` (`enforce_fs_policy`, `emit_fs_event`) | [T20260419-0503] |
| Activity `fsProfile:` binding | `crates/orbit-engine/src/activity_job/{dispatcher,job_executor,agent_loop_driver,groundhog}.rs` | [T20260419-0503] |
| Exec spawn primitive | `crates/orbit-exec/src/{lib,runner,process,sandbox}.rs` | [T20260417-0550] |
| Process supervision | `crates/orbit-exec/src/supervision/{wait,cleanup,signal,tee}.rs` | [T20260417-0558-4], [T20260417-0558-5] |
| Filesystem denial audit channel | `crates/orbit-tools/src/lib.rs` (`FsAuditLogger`) → `docs/design/auditability/2_design.md §3` | [T20260426-0605] |

---

## Task References

- **[T20260416-0728]** — Align policy contract with runtime enforcement (v2 schema, effective profile resolution).
- **[T20260417-0550]** — Decompose `orbit-exec` supervision modules.
- **[T20260417-0558-4]** / **[T20260417-0558-5]** — Harden `orbit-exec` supervision (signal pipe, process-group reaping).
- **[T20260419-0503]** — Enforce `fsProfiles` across runtime and CLI.
- **[T20260426-0605]** — Add the auditability design folder cross-linked from §3.
- **[T20260426-0622]** — Add this policy & sandboxing design folder under claude ownership.
- **[T20260430-23]** — Shorten the policy sandbox design docs while preserving the shipped contract and ADR history.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
