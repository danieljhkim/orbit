# Policy & Sandboxing — Vision

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-16

This document captures the questions Orbit must answer before policy and sandboxing become a fuller safety contract. [2_design.md](./2_design.md) describes today's implementation; this file keeps future work distinct from shipped guarantees.

---

## 1. Open Questions

1. **Should `orbit-exec` get real `Sandbox` impls?** `NoSandbox` is still the default; candidates include `bubblewrap`, containers, seccomp, and platform wrappers.
2. **Should enforcement move below the tool layer?** A future tool that skips `enforce_fs_policy` is unguarded unless Orbit adds a `PolicyAwareFs` trait, syscall interception, or linting.
3. **Should `proc.spawn` consult policy?** Activity program allowlists are not `PolicyDef`; future shapes include `allowExec` / `denyExec` or env access tied to `fsProfile`.
4. **What is the symlink contract?** `workspace_relative_path` follows symlinks and denies out-of-workspace targets, but the invariant is not yet specified.
5. **Should glob syntax grow?** Character classes, braces, and broader `**` forms would reduce user surprise but may re-evaluate existing profiles differently.
6. **Should `PolicyDecision` and `FsPolicyEvaluation` converge?** A unified outcome could serve future network, exec, and env policy checks.
7. **Should profiles be composable?** `extends:`, `includes:`, or mixins would reduce repetition but add resolution-order questions.
8. **Should empty rule lists warn?** `read: []` / `modify: []` safely denies everything, but a load-time warning would catch likely mistakes earlier.
9. **What is the dry-run / explain story?** A command like `orbit policy explain --profile <name> --op modify --path <path>` would shorten policy authoring loops.
10. **Should all denials share one audit shape?** Fs denials, task-lock denials, program allowlists, and future exec denials still report through different channels; auditability asks the same question.
11. **How should concurrent exec handle signals?** `SignalHandlerGuard` serializes installs; worker-pool exec may need sigmasks, cancellation tokens, or a supervisor thread.
12. **How far should CLI policy coverage go?** macOS `sandbox-exec` narrows writes, but alternatives include trapping CLI fs calls or moving more work to HTTP-backed activities.

---

## 2. Prior Work

### 2.1 Orbit-Internal

The [activity-job audit-envelope spec](../activity-job/specs/audit-envelope.md) defines how filesystem and tool denials surface as `V2AuditEvent` entries. The auditability folder ([../auditability/2_design.md §3](../auditability/2_design.md)) documents durable storage.

The current policy schema and merge contract live in `crates/orbit-common/src/types/policy_def.rs` and `crates/orbit-common/src/types/resource.rs`.

### 2.2 OS-Level Sandboxes

`bubblewrap`, `sandbox-exec`, `firejail`, and seccomp-bpf are the near-term isolation options under the `Sandbox` trait. gVisor and Firecracker are heavier options when a workload tolerates a microVM boundary.

### 2.3 Capability Systems

POSIX capabilities, Capsicum, and Linux Landlock express process rights as capabilities rather than path globs. Landlock is attractive because it is hierarchical and works without root, but it is Linux-only.

### 2.4 Build Sandboxes

Bazel `exec.sandbox`, Buck2 hermetic execution, and Nix sandboxing treat the workspace as a closed input set. They are stricter than Orbit's allowlist-plus-global-deny model.

### 2.5 Process Supervision Patterns

`tini`, `dumb-init`, and Kubernetes termination grace model the same SIGTERM-then-SIGKILL escalation that `orbit-exec` implements. Per-activity grace periods are a plausible future extension.

---

## 3. What May Be Distinctive

1. **Activity-bound profiles.** Every activity declares its profile, and the resolver re-evaluates per call.
2. **Project-shaped globs.** Profiles use paths such as `./src/**`, trading capability precision for readable project intent.
3. **Global negative denies.** `denyRead` / `denyModify` inject into every resolved profile; no profile opts out of them locally.
4. **Auditable by construction.** HTTP fs decisions emit events as part of `enforce_fs_policy`.
5. **Workspace-relative resolution.** Profiles stay portable because paths are evaluated relative to the active workspace.

---

## 4. References

Orbit-internal:

- [1_overview.md](./1_overview.md) — feature purpose and concept map.
- [2_design.md](./2_design.md) — shipped implementation and limitations.
- [specs/fs-profile-resolution.md](./specs/fs-profile-resolution.md) — prescriptive resolution and evaluation contract.
- [specs/sandbox-exec-contract.md](./specs/sandbox-exec-contract.md) — exec spawn and supervision contract.
- [../auditability/2_design.md](../auditability/2_design.md) — how policy denials surface to durable audit.
- [../activity-job/2_design.md](../activity-job/2_design.md) — how activities thread `fsProfile:` through dispatch.

External reference categories:

- OS-level sandboxes: bubblewrap, sandbox-exec, firejail, seccomp-bpf, gVisor, Firecracker.
- Capability systems: POSIX capabilities, Capsicum, Linux Landlock.
- Build sandboxes: Bazel exec.sandbox, Buck2 hermetic execution, Nix build sandbox.
- Supervision patterns: tini, dumb-init, Kubernetes terminationGracePeriodSeconds.

---

## Task References

- **[T20260416-0728]** — Established the v2 policy contract that this document extends.
- **[T20260419-0503]** — Made `fsProfiles` enforcement runtime-wide.
- **[T20260417-0558-4]** / **[T20260417-0558-5]** — Hardened the supervision contract that §1.11 wants to evolve.
- **[T20260426-0605]** — Auditability folder linked from §1.10.
- **[T20260426-0622]** — Add this folder and name the open questions.
- **[T20260430-23]** — Shorten the policy sandbox design docs while preserving the shipped contract and ADR history.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
