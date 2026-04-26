# Policy & Sandboxing — Vision

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-04-26

This document captures the questions that have to be answered before Orbit's policy and sandboxing story matches a defensible long-term safety contract. [2_design.md](./2_design.md) describes today's implementation; this file names the pressure points that should drive future tasks and ADRs.

---

## 1. Open Questions

1. **Should `orbit-exec` get a real `Sandbox` impl?** The trait has shipped, but only `NoSandbox` is registered. Plausible candidates are `bubblewrap` on Linux, `sandbox-exec` on macOS, container-based isolation, or syscall filtering via seccomp. The cost is platform-specific code and a packaging story; the benefit is genuine OS-level isolation that does not depend on tool authors routing through `enforce_fs_policy`.
2. **Should policy enforcement move below the tool layer?** Today `enforce_fs_policy` is called from `orbit-tools::builtin::fs`. A future tool that does fs work without that helper is unguarded. Options: a `PolicyAwareFs` trait that every tool must use; a runtime that intercepts at the syscall layer; or a static lint that rejects unguarded fs calls. Each has a different trust model.
3. **Should `proc.spawn` consult the policy engine?** Today exec is gated only by activity-level program allowlists, not by `PolicyDef`. Adding `denyExec` / `allowExec` to the schema is one shape; binding `fsProfile` to also constrain `proc.spawn` env access is another.
4. **What is the symlink contract?** Today `workspace_relative_path` canonicalizes and falls back to `OrbitError::PolicyDenied("path is outside workspace")` when a symlink resolves out of the workspace. This is conservative but un-documented. The contract should explicitly state whether symlinks are followed, when a symlink-out denial is distinguishable from a normal deny, and whether profile rules can match against the unresolved path.
5. **Should the glob translator grow to match user expectations?** Profile authors often expect character classes, brace expansion, and `**` at the start of a pattern. Today the translator supports `*`, `**`, `?`, and `<prefix>/**`. Expanding it has compatibility cost (existing profiles get re-evaluated) but reduces "why didn't my pattern match" friction.
6. **Should `PolicyDecision` and `FsPolicyEvaluation` converge?** Two parallel allow/deny shapes is not sustainable as the policy surface grows beyond fs. A unified `PolicyOutcome { allowed, decision_kind, matched_rule, reason }` would let future evaluators (network, exec, env) plug in without picking sides.
7. **Should profiles be composable?** Today a profile is a flat `{ read, modify }` declaration, and merging is policy-level (workspace overrides global). Composition (`extends:`, `includes:`, mixin profiles) would make policy authoring less repetitive but introduces resolution-order questions.
8. **Should empty rule lists deny silently or emit a configuration warning?** Today an `FsProfile { read: [], modify: [] }` denies everything with sentinel `"[]"`. That is technically correct but is almost always a configuration mistake, not an intentional safety stance. A load-time warning would surface the misconfiguration earlier than the first deny.
9. **What is the dry-run / explain story?** Profile authors today have to write a profile, run an activity, and read the audit log to discover whether a path is allowed. A `orbit policy explain --profile <name> --op modify --path <path>` command would shorten the loop without changing runtime semantics.
10. **Should policy denials always produce structured audit?** Today denials emit through `FsAuditLogger` for fs and through targeted command-audit rows for task locks, but other denial classes (program allowlist, gate starvation, future exec denials) do not share one schema. Cross-link to [auditability §3.1 open questions](../auditability/3_vision.md#1-open-questions) — uniform denial shape is named there too.
11. **How should signal handler installation behave under concurrent exec?** `SignalHandlerGuard` serializes via a global `Mutex`. If exec ever moves into a worker pool, this becomes a contention point. Options: per-thread sigmask manipulation, cooperative cancellation tokens that bypass signal handling for non-foreground exec, or a single supervisor thread that owns all child supervision.
12. **How should the CLI backend gain policy coverage?** Today `backend: cli` activities spawn an external CLI agent that owns its own filesystem behavior, so `enforce_fs_policy` never runs and `fsProfile:` is informational. Closing this is structurally hard: options include (a) wrapping CLI runtimes in an OS-level sandbox so the harness cannot escape the profile even when Orbit doesn't intercept its calls, (b) routing CLI fs through a trapping shim that re-enters the policy engine, or (c) deprecating `backend: cli` in favor of HTTP-only activities. Each has a different cost and migration story.

---

## 2. Prior Work

### 2.1 Orbit-Internal

The closest internal work is the [activity-job audit-envelope spec](../activity-job/specs/audit-envelope.md), which defines how filesystem and tool denials surface as `V2AuditEvent` entries. The auditability folder ([../auditability/2_design.md §3](../auditability/2_design.md)) documents how those events reach durable storage.

The current policy schema and merge contract are anchored in `crates/orbit-common/src/types/policy_def.rs` and `crates/orbit-common/src/types/resource.rs`. They give the implementation a single seam at which a future evaluator can plug in.

### 2.2 OS-Level Sandboxes

`bubblewrap` (Linux user-namespace sandbox), `sandbox-exec` (macOS Seatbelt), `firejail` (SUID-based Linux sandbox), and seccomp-bpf filters are the credible options for adding genuine isolation under the existing `Sandbox` trait. Each has different platform coverage, packaging cost, and side-effect surface (e.g., file mounts vs. syscall filters). gVisor and Firecracker are heavier but offer stronger isolation when the workload tolerates a microVM boundary.

### 2.3 Capability Systems

POSIX capabilities, Capsicum (FreeBSD), and Linux Landlock all express "what may this process do" as orthogonal capability bits rather than path globs. Landlock in particular is a credible long-term destination: it is per-thread, hierarchical, and works without root. The cost is Linux-only coverage; the benefit is that capabilities apply to every fs syscall rather than only the calls a tool routes through Orbit.

### 2.4 Build Sandboxes

Bazel `exec.sandbox`, Buck2's hermetic execution, and the Nix build sandbox model treat the workspace as a closed input set and reject any path not listed. They are stricter than Orbit's "allow within profile, deny within global denies" model but show that path-level enumeration scales to large repositories when paired with a fast index.

### 2.5 Process Supervision Patterns

`tini`, `dumb-init`, and Kubernetes' termination-grace-period contract all model the same SIGTERM-then-SIGKILL escalation that `crates/orbit-exec/src/supervision/cleanup.rs` implements. The current 5-second grace is consistent with those references; tuning that constant per activity (similar to a Kubernetes preStop hook) is a credible future request.

---

## 3. What May Be Distinctive

1. **Profile-scoped, activity-bound.** Most sandboxes are process-bound or workload-bound. Orbit policy is activity-bound: every activity declares the profile it runs under, and the resolver re-evaluates per call. That makes it natural to express "this single tool call runs under a tighter profile" without spawning a new process.
2. **Globs, not capability bits.** Profile authors write `./src/**` instead of opening file descriptors with bounded capabilities. The cost is that enforcement is path-string-shaped; the benefit is that profiles read like project-level intent.
3. **Globally negative denies.** `denyRead` / `denyModify` always inject as negated rules into every resolved profile. There is no profile that can opt out of a global deny without policy ownership. This is unusual; most allowlist systems treat denies as a separate evaluation pass.
4. **Auditable by construction.** Every fs decision generates an event regardless of allow/deny outcome (as long as `fs_audit` is wired). The audit story is not bolted on after the fact; it is part of `enforce_fs_policy`.
5. **Workspace-relative resolution.** Paths are evaluated workspace-relative, not absolutely. Two different workspaces can have the same profile and produce different absolute paths in their audits, which keeps profile authoring portable.

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

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
