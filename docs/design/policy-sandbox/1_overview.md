# Policy & Sandboxing — Overview

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-04-26

Policy & Sandboxing is the safety surface that decides what an agent is allowed to read, modify, and execute inside a workspace. It is two layers stitched together: a filesystem-scoping policy engine that resolves a named `FsProfile` plus global `denyRead` / `denyModify` rules into an allow/deny answer for every fs operation, and a process supervision layer that spawns shell commands with timeouts, signal handling, and process-group cleanup. [2_design.md](./2_design.md) describes the shipped implementation; [3_vision.md](./3_vision.md) names the gaps between today's policy/sandbox semantics and a defensible long-term contract.

---

## 1. Motivation

Orbit runs agents against user repositories, so the safety boundary is a product feature rather than an internal hygiene concern.

1. **Default-safe with explicit expansion.** When an activity does not declare an `fsProfile:`, runtime materializes an implicit `unrestricted` profile so the activity still runs against `./**`, but every read and modify still passes through the same evaluator and audits the same way. There is no silent bypass path.
2. **Profiles are activity-scoped.** Each activity binds to one named profile. Two activities in the same job can run under different profiles, and the resolver re-evaluates per call. Profile switching happens at activity boundaries, not inside a tool call.
3. **Deny rules are global, not profile-local.** `denyRead` and `denyModify` live on the policy itself and are injected into every resolved profile as negated rules. A workspace-level policy can only narrow the surface; it cannot widen past a global deny.
4. **Sandboxing is shell supervision, not OS isolation.** `orbit-exec` ensures that every spawned process is reaped, time-bounded, and signal-aware. It is not a kernel-level sandbox today. The real isolation surface is the policy/tool layer, which means tool authors — not the exec runner — are responsible for routing fs work through the policy engine.
5. **Denials must be auditable.** Filesystem denials emit through `FsAuditLogger` and surface as `V2AuditEvent` filesystem entries. Cross-link to [docs/design/auditability/](../auditability/) for how those records are stored.

---

## 2. Core Concepts

### 2.1 Policy is a v2 schema with named profiles and global denies

`PolicyDef` declares `denyRead`, `denyModify`, and a map of named `FsProfile` entries. Each profile lists `read` and `modify` glob rules. Schema version 1 is rejected at load time; only v2 with the three sections is accepted. Workspace policies override globals by name and concatenate global denies.

### 2.2 Profile resolution materializes an implicit `unrestricted`

When an activity omits `fsProfile:`, the v2 host substitutes the constant `UNRESTRICTED_FS_PROFILE`. If no policy defines `unrestricted`, the policy engine fills in `read: ["./**"]` and `modify: ["./**"]` so the lookup never fails. Global denies are then injected as negated rules, so even an unrestricted activity cannot read or modify a globally denied path.

### 2.3 Path evaluation is last-match-wins over a normalized rule list

`PolicyDef::check_path` walks the resolved rule list in order. Each rule is either positive (`./src/**`) or negated (`!./.git/**`). The last rule that matches the normalized workspace-relative path wins. If no positive rule exists, every path is denied with a sentinel `[]` matched-rule string. If positive rules exist but none match, the denial returns `<no matching rule>`.

### 2.4 Tool-layer enforcement applies to HTTP-backed activities only

The `orbit-tools` `fs.*` builtins call `enforce_fs_policy` before every read or modify. The policy decision is rendered into an `FsCallEvent` (request, result, or denied) and emitted through the activity's `FsAuditLogger`. A denied call returns `OrbitError::PolicyDenied` and never reaches the filesystem. The exec layer does not consult policy directly — it trusts the calling tool to have already applied it.

This enforcement reaches only the HTTP agent loop. `backend: cli` activities spawn an external CLI agent (Claude Code, Codex CLI, etc.) that owns its own filesystem behavior and does not route through `enforce_fs_policy`. For those activities, Orbit records a `tool_allowlist.harness_delegated` envelope event and trusts the harness; `fsProfile:` is informational, not enforced. See [2_design.md §9](./2_design.md#9-concerns--honest-limitations).

### 2.5 Sandboxed exec is process supervision, not isolation

`orbit-exec::run_process` spawns a child as a process-group leader, drains stdout/stderr in background threads to prevent pipe-fill deadlocks, installs SIGINT/SIGTERM handlers in the parent, and on timeout or signal sends SIGTERM to the entire process group with a 5 second grace period before SIGKILL. The default `Sandbox` impl is `NoSandbox`; the trait is in place for future kernel-level isolation but is not implemented today.

---

## 3. At a Glance

| Concern | Where it lives | Primary task ID |
|---------|----------------|-----------------|
| Policy schema and validation | `crates/orbit-common/src/types/policy_def.rs`, `crates/orbit-common/src/types/resource.rs` | [T20260416-0728] |
| Allow/deny enum | `crates/orbit-common/src/types/policy_decision.rs` | [T20260426-0622] |
| Policy facade | `crates/orbit-policy/src/{lib,engine,evaluator,decision}.rs` | [T20260416-0728] |
| Profile resolution + deny injection | `crates/orbit-common/src/types/policy_def.rs` (`effective_profile`, `check_path`) | [T20260416-0728] |
| Implicit `unrestricted` materialization | `crates/orbit-core/src/runtime/v2_host.rs` (`tool_context_for_activity`) | [T20260419-0503] |
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

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
