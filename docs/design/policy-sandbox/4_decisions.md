# Policy & Sandboxing — Decisions

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-04-26

This is the append-only ADR log for Policy & Sandboxing. Entries are ordered by ADR number. New entries follow the template in [../CONVENTIONS.md](../CONVENTIONS.md) and cite the task that made the decision real.

---

## ADR-001 — Dedicated policy & sandboxing design ownership

**Status:** Accepted · 2026-04 · [T20260426-0622]

**Context.** Policy and sandboxing semantics were spread across `orbit-policy`, `orbit-exec`, the `PolicyDef` schema in `orbit-common`, the activity dispatcher, and the v2 host. There was no canonical place to record invariants, the `unrestricted` fallback, or the supervision contract.

**Decision.** Create `docs/design/policy-sandbox/` as the canonical design folder, with claude as owner. Auditability owns the recording of denials; this folder owns the *semantics* of allow/deny and the *contract* for how spawned processes are supervised.

**Consequences.**
- Policy and sandboxing decisions now have one ADR log and one glossary.
- Future enforcement work can cite a feature-owned spec rather than re-deriving rules from code.
- Cost: this folder cross-links into auditability and activity-job, so when those folders change their cross-references must be kept in sync rather than this folder absorbing them.

## ADR-002 — Policy schema is v2-only with named profiles plus global denies

**Status:** Accepted · 2026-04 · [T20260416-0728]

**Context.** An earlier policy schema (v1) used a different shape for allow/deny rules. Supporting both shapes in the runtime caused interpretation drift between the loader, the merger, and the evaluator.

**Decision.** Reject `schemaVersion: 1` at load time with an explicit migration message. v2 declares `denyRead`, `denyModify`, and `fsProfiles` and is the only accepted shape. Workspace policies override globals by profile name; global denies accumulate.

**Consequences.**
- Schema parsing has one supported branch.
- Profile authoring becomes uniform: a profile is always a `{ read, modify }` declaration; denies are always global.
- Cost: existing v1 policy files require a manual migration; there is no automatic upgrader.

## ADR-003 — Implicit `unrestricted` profile materializes when an activity omits `fsProfile:`

**Status:** Accepted · 2026-04 · [T20260419-0503]

**Context.** Activities can omit `fsProfile:`. A naive design would either reject the activity at load or run it without policy enforcement. Both are wrong: rejection breaks the common case, and unguarded execution means audit blindness.

**Decision.** When an activity omits `fsProfile:`, the v2 host substitutes the constant `UNRESTRICTED_FS_PROFILE` ("unrestricted") at `tool_context_for_activity`. If the policy does not define a profile of that name, the resolver synthesizes `read: ["./**"]` and `modify: ["./**"]`. Global `denyRead` / `denyModify` rules still apply because they are injected after profile resolution.

**Consequences.**
- "Unrestricted" is the default, but it is still narrowed by global denies.
- Policy authors can shadow the implicit fallback by declaring a profile named `unrestricted`, which is the supported way to narrow the default.
- Audit emission still happens for unrestricted activities, so there is no silent unguarded path.
- Cost: the word "unrestricted" carries different meaning depending on whether the policy defines a profile of that name, which is a learnable but real source of confusion.

## ADR-004 — Deny rules inject as negated profile rules with last-match-wins evaluation

**Status:** Accepted · 2026-04 · [T20260416-0728]

**Context.** A separate "deny pass" before profile evaluation is the obvious shape, but it makes precedence ambiguous when a profile rule and a deny rule both match. Multiple Orbit features (workspace overrides, profile narrowing, denyModify-also-implies-denyRead-for-modify validation) need a single evaluation order.

**Decision.** `effective_profile` appends every entry of `denyRead` to the profile's `read` list as `!<rule>` and every entry of `denyModify` to the profile's `modify` list as `!<rule>`. `check_path` walks the resolved list in order and the **last match wins**. There is no separate deny pass.

**Consequences.**
- Profile rules and deny rules are evaluated in one pass against one list.
- Deny ordering is deterministic: denies are appended after profile rules, so they always win against an earlier positive match for the same path.
- Cost: a profile author cannot re-allow a globally denied path by ordering, which is the intended safety property but surprises authors who expect a simple allowlist with overrides.

## ADR-005 — Modify rules must be covered by a read rule in the same profile

**Status:** Accepted · 2026-04 · [T20260416-0728]

**Context.** A profile that grants `modify: ["./build/**"]` without granting `read: ["./build/**"]` is technically valid but produces a confusing operational story: a tool may be allowed to write a file it cannot read, breaking the standard read-modify-write pattern.

**Decision.** `PolicyDef::validate` rejects any profile whose positive `modify` rule is not covered by a positive `read` rule in the same profile. "Covered" is checked structurally (`rule_covers_path_rule`): exact match, `**`, or a `<prefix>/**` rule that prefixes the modify rule.

**Consequences.**
- Modify rules cannot exist without a corresponding read rule.
- The audit story is consistent: every modify implies a prior allowed read for the same path.
- Cost: profile authors who *only* want to allow append-style writes cannot express that without granting a read rule. There is no "write-only" profile shape today.

## ADR-006 — Tool layer is the policy enforcement point for HTTP-backed activities

**Status:** Accepted · 2026-04 · [T20260419-0503]

**Context.** Policy enforcement could plausibly live at the syscall layer, the fs trait layer, the tool layer, or the activity layer. Each placement has different trust and coverage tradeoffs.

**Decision.** Enforcement lives in `orbit-tools::builtin::fs::enforce_fs_policy`. Every fs builtin calls it before the underlying read or modify, and emits `FsCallEvent` through `FsAuditLogger`. The `Sandbox` trait in `orbit-exec` does not consult the policy engine; exec is supervised but not policy-gated. This applies only to `backend: http` activities — `backend: cli` runs spawn an external CLI agent and emit a `tool_allowlist.harness_delegated` event in lieu of enforcement.

**Consequences.**
- HTTP-backed activities have a single, auditable enforcement seam.
- Tool authors are responsible for routing fs work through the helper. The contract is small, but it is a discipline rather than a structural invariant.
- The audit story has one shape — every HTTP-backed fs decision flows through one helper that emits one event family.
- Cost: CLI-backed activities are entirely unenforced by Orbit; a future tool (or a non-builtin tool inside HTTP) that performs fs work without using the helper is also unguarded. Both gaps are named in [2_design.md §9](./2_design.md#9-concerns--honest-limitations); closing them likely requires a `PolicyAwareFs` trait, an OS-level sandbox under CLI runtimes, or both.

## ADR-007 — Children spawn as process-group leaders so orphan subprocesses are reapable

**Status:** Accepted · 2026-04 · [T20260417-0558-4], [T20260328-221810]

**Context.** Naive subprocess code on Unix leaves orphan grandchildren holding open pipe write ends, which causes the parent's `wait_with_output` to hang when the orphan never exits. Earlier versions of orbit-exec hit this exact failure when an agent's tool spawned long-lived helpers.

**Decision.** On Unix, every spawned child calls `command.process_group(0)` so the child becomes a process-group leader (PGID = PID). The supervision layer kills the entire group via `killpg` when the child exits, when the parent receives SIGINT/SIGTERM, or when the deadline expires.

**Consequences.**
- Orphan subprocesses are reaped, so `wait_with_output` no longer hangs.
- Signal handling can target the whole tree with one syscall.
- Cost: tools that intentionally fork detached helpers (e.g., long-running daemons) cannot do so under orbit-exec without explicitly creating their own process group inside the child.

## ADR-008 — SIGTERM with 5-second grace, then SIGKILL for the whole group

**Status:** Accepted · 2026-04 · [T20260417-0558-4]

**Context.** A timed-out or interrupted child needs a chance to flush state before being killed, but the supervisor cannot wait indefinitely. The escalation policy needs a single, predictable shape.

**Decision.** `terminate_process_group` sends `SIGTERM` (or the supplied signal) to the group, polls `process_group_is_alive` for `TERMINATION_GRACE_PERIOD = 5 seconds`, and on expiry sends `SIGKILL` to the group plus a direct `child.kill()`/`child.wait()`. stderr is annotated with `process timed out` (deadline path) or `process interrupted by signal SIG…` (parent-signal path).

**Consequences.**
- Termination is deterministic: at most 5 seconds between intent and SIGKILL.
- The annotated stderr lets audit consumers distinguish timeout vs. signal vs. clean exit without reading the exit code alone.
- Cost: the 5-second constant is global. Activities that need a longer drain (database flush, large I/O cleanup) cannot extend it without code changes.

## ADR-009 — Signal handler installation is process-global and serialized

**Status:** Accepted · 2026-04 · [T20260417-0558-5]

**Context.** Installing parent-side SIGINT/SIGTERM handlers is a process-global operation. Two concurrent `run_process` calls cannot install independent handlers without races, and a panicking call must restore the prior handler so the orbit process itself remains interruptible.

**Decision.** `SignalHandlerGuard::install` acquires a `Mutex` from a `OnceLock`, creates a non-blocking pipe, calls `libc::sigaction` for SIGINT and SIGTERM, and stores the previous `sigaction` structs. Drop reverses the steps: restore previous handlers, close the pipe, release the mutex. The handler itself is async-signal-safe (atomic load + 1-byte `write`).

**Consequences.**
- Concurrent `run_process` calls in the same process serialize on the mutex during install/drop, but the wait loops themselves run concurrently.
- A panic inside a wait loop still restores prior handlers via Drop.
- Cost: contention on the global mutex limits exec parallelism in a single process. Named as an open question in [3_vision.md §1.11](./3_vision.md#1-open-questions).

## ADR-010 — `NoSandbox` is the default `Sandbox` impl; real isolation is deferred

**Status:** Accepted · 2026-04 · [T20260417-0550]

**Context.** The `Sandbox` trait is the seam where kernel-level or container-level isolation would attach to `orbit-exec`. The trait shipped with the supervision rework, but no real impl is registered.

**Decision.** Ship `NoSandbox` as the default and only implementation. Defer kernel-level isolation (bubblewrap, sandbox-exec, container, seccomp) until policy enforcement at the tool layer is judged insufficient and the platform-coverage cost is understood. The trait surface is stable so a future impl can attach without changing the runner.

**Consequences.**
- The trait surface is stable for a future impl to attach to.
- Today's safety story is "policy at the tool layer" — explicit and documented, but not OS-enforced.
- Cost: a tool that performs fs work without `enforce_fs_policy` (or a future non-builtin tool) has no exec-level isolation backstop. This is the structural reason §1.1 of [3_vision.md](./3_vision.md) lists real sandboxing as the top open question.

---

## Task References

- **[T20260328-221810]** — Subprocess termination on Ctrl+C / job cancel; predecessor of the current process-group design.
- **[T20260416-0728]** — Aligned the policy contract with runtime enforcement; v2 schema and effective-profile resolution land here.
- **[T20260417-0550]** — Decomposed `orbit-exec` supervision modules.
- **[T20260417-0558-4]** / **[T20260417-0558-5]** — Hardened `orbit-exec` supervision (process-group reaping, signal-pipe handler).
- **[T20260419-0503]** — Enforced `fsProfiles` across runtime and CLI; introduced `tool_context_for_activity`.
- **[T20260426-0622]** — Add this design folder and record the initial ADR set.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
