# Policy & Sandboxing — Design

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-04-26

This document describes Orbit's shipped policy and sandboxing implementation: the v2 `PolicyDef` schema, profile resolution and the implicit `unrestricted` fallback, deny-rule injection, last-match-wins path evaluation, the `orbit-policy` engine facade, tool-layer enforcement in `orbit-tools`, the activity/job binding that threads an `fsProfile` through every dispatcher path, the `orbit-exec` spawn primitive, and the process supervision layer that handles timeouts, signals, and orphan reaping. See [1_overview.md](./1_overview.md) for purpose and [3_vision.md](./3_vision.md) for forward-looking gaps.

---

## 1. Policy Schema

The policy schema lives on `PolicyDef` in `crates/orbit-common/src/types/policy_def.rs`. A v2 policy declares:

- `name`
- optional `description`
- `denyRead` — global read-denial rules applied to every profile
- `denyModify` — global modify-denial rules applied to every profile
- `fsProfiles` — a map of profile name to `FsProfile { read, modify }`

`crates/orbit-common/src/types/resource.rs` rejects `schemaVersion: 1` at load time with an explicit migration message ("policy schemaVersion 1 is no longer supported; migrate to schemaVersion 2 with `spec.denyRead`, `spec.denyModify`, and `spec.fsProfiles`"). v2 is the only accepted shape.

`PolicyDef::validate` enforces three structural invariants beyond rule normalization:

1. Every profile name is non-empty.
2. A profile's `modify` rule must be covered by at least one `read` rule in the same profile. The validator rejects modify-only writeable paths because a tool needs to read a path to make a meaningful modification, and granting modify without read produces a confusing audit story.
3. A profile rule that exactly duplicates a global `denyRead` or `denyModify` entry is rejected. Profiles can use negations to refine globals, but cannot copy a denied rule and effectively re-allow it by ordering.

`PolicyDef::merged(global, workspace)` is the merge contract: workspace `fsProfiles` overwrite global entries by name, while global `denyRead` / `denyModify` accumulate (workspace cannot remove a global deny). Merging always re-runs `validate`.

---

## 2. Profile Resolution

`PolicyDef::effective_profile(profile_name)` returns a `ResolvedFsProfile { name, read, modify }` after applying three transformations:

1. **Lookup.** If `fs_profiles` contains `profile_name`, that profile's rule lists are the base. If not and `profile_name == "unrestricted"`, the resolver synthesizes `FsProfile { read: ["./**"], modify: ["./**"] }`. Any other missing profile name returns `OrbitError::InvalidInput`.
2. **Normalization.** Each rule is trimmed, backslashes are flipped to forward slashes, leading `./` segments are stripped, and the rule is rejected if it escapes the workspace (`~`, `~/`, absolute paths, parent traversals). Rules also compile to a glob-equivalent regex for evaluation.
3. **Deny injection.** Every entry in `denyRead` is appended to the profile's `read` list as a negated rule (`!<rule>`); every entry in `denyModify` is appended to the profile's `modify` list as a negated rule. This is the mechanism that makes deny rules global without a separate evaluation pass.

The implicit `unrestricted` profile only materializes if both conditions hold: the activity omitted `fsProfile:` *and* the policy did not define a profile named `unrestricted`. A policy author who defines `unrestricted` shadows the implicit fallback, which is the intended escape hatch for narrowing a workspace's "unrestricted" mode.

---

## 3. Path Evaluation

`PolicyDef::check_path(profile, op, path)` returns an `FsCheckResult { allowed, matched_rule }`. The algorithm:

1. Resolve the profile (via §2).
2. Pick the rule list by operation (`read` or `modify`).
3. If the list is empty, deny with `matched_rule = "[]"`.
4. Walk every rule in order, recording the most recent match. The rule is split into negated/positive form, compiled to a regex, and matched against the normalized workspace-relative path. Later matches override earlier ones — this is **last-match-wins**, not first-match.
5. After the walk, if any rule matched, the decision uses the last match's negation flag. If no rule matched but the list contained at least one positive rule, deny with `matched_rule = "<no matching rule>"`. If the list contained only negated rules, treat that as an empty positive set and deny with `matched_rule = "[]"`.

Path normalization (`normalize_path`) trims, flips slashes, strips `./` prefixes, and rejects absolute paths or `~`-anchored paths. Tool callers are expected to canonicalize first and then express the path workspace-relative — `crates/orbit-tools/src/builtin/fs/mod.rs::workspace_relative_path` handles that on the call site.

The glob-to-regex translator supports `*` (single-segment wildcard), `**` (cross-segment wildcard), `?` (single character within a segment), and `<prefix>/**` (directory subtree match). The translator is intentionally narrow; it is not a full POSIX glob.

---

## 4. PolicyEngine Facade

`crates/orbit-policy/src/lib.rs` re-exports `PolicyEngine`, `FsPolicyEvaluation`, and `PolicyDecision`. The engine wraps a validated `PolicyDef` and exposes one operational method:

```
PolicyEngine::check(profile, operation, path) -> FsPolicyEvaluation
```

`FsPolicyEvaluation` is the struct callers receive: `{ profile, operation, path, allowed, matched_rule }`. The evaluator module (`evaluator.rs`) is a thin pass-through to `PolicyDef::check_path`; the indirection exists so future evaluators (e.g. caching, layered profiles) can replace the implementation without changing the engine surface.

`PolicyDecision` (`crates/orbit-common/src/types/policy_decision.rs`) is a separate `Allow | Deny { reason }` enum used by the broader policy/RBAC framing. It is currently a re-export from `orbit-policy::decision` and is not generated by `PolicyEngine::check` — that path returns the richer `FsPolicyEvaluation` instead. The decision enum exists for non-fs callers and for future allow/deny answers that do not carry `matched_rule` semantics.

---

## 5. Tool-Layer Enforcement

`crates/orbit-tools/src/builtin/fs/mod.rs::enforce_fs_policy` is the only place fs operations consult the policy engine today. The flow:

1. Read `ctx.fs_profile` (the profile name string set by the runtime when constructing the `ToolContext`).
2. Read `ctx.policy_engine` (the `Arc<PolicyEngine>` set by the runtime).
3. If either is missing, return `Ok(None)` — fs work proceeds unguarded. This is the "no policy installed" path used in unit tests and is not reachable from a real v2 host.
4. Convert the canonical filesystem path to a workspace-relative form (`workspace_relative_path`).
5. Call `policy_engine.check(profile, op, path)`.
6. Build an `FsPolicyAllowance` carrying `{ profile, op, path, matched_rule }`.
7. If allowed, emit `FsCallEvent { kind: Request, allowed: true, ... }` through `ctx.fs_audit` and return the allowance so the caller can later emit `FsCallEvent::Result`.
8. If denied, emit `FsCallEvent { kind: Denied, allowed: false, ... }` and return `OrbitError::PolicyDenied("fs.<op> denied for <path> under fsProfile <profile> (matched rule <rule>)")`.

The audit emission goes through `ctx.fs_audit: Option<Arc<dyn FsAuditLogger>>` (`crates/orbit-tools/src/lib.rs`). The v2 dispatcher wires this to `v2_fs_audit_logger(audit.clone())`, which converts each `FsCallEvent` into a `V2AuditEvent` filesystem entry. The full audit-channel description belongs to [auditability](../auditability/2_design.md#3-tool-driven-and-runtime-audit-records); this folder owns the *enforcement* contract, not the storage contract.

`FsCallEvent` itself carries `{ kind, profile, op, path, allowed, matched_rule }`. There is no separately persisted negation flag — `allowed = false` is the only structural signal that a match was a deny. Consumers that need to distinguish "denied by an explicit deny rule" from "denied because no rule matched" must compare `matched_rule` against the policy's deny lists themselves.

The exec layer does not consult the policy engine at all. There is no `proc.spawn` policy gate today — exec is sandboxed only by what the calling tool has already validated and by the supervision contract in §8.

**Backend scope.** Tool-layer enforcement only fires when an activity runs under `backend: http` and reaches an Orbit fs builtin. `backend: cli` activities spawn an external CLI agent (Claude Code, Codex CLI, etc.) via `cli_runner.rs`; that path emits a `tool_allowlist.harness_delegated` envelope event and trusts the harness for fs behavior. The `fsProfile:` field is still recorded on the activity, but it does not gate fs access for CLI-backed runs because no Orbit fs builtin executes inside that subprocess. See [§9](#9-concerns--honest-limitations) for the resulting coverage gap.

---

## 6. Activity / Job fsProfile Binding

The `fsProfile:` field on an activity flows through `crates/orbit-engine/src/activity_job/`:

- `dispatcher.rs` carries `fs_profile: Option<&str>` on `DispatchInput` and threads it into `run_activity_job_dispatch`, `run_loop_step_dispatch`, and `run_agent_loop_via_driver`.
- `job_executor.rs` reads `t.fs_profile.as_deref()` from the activity spec at the call site of every step type.
- `agent_loop_driver.rs` and `groundhog.rs` invoke `host.tool_context_for_activity(fs_profile, audit_logger)` to construct the `ToolContext` that fs builtins read from.

`crates/orbit-core/src/runtime/v2_host.rs::tool_context_for_activity` is the single materialization point:

```
fs_profile: Some(fs_profile.unwrap_or(UNRESTRICTED_FS_PROFILE).to_string())
```

This is the implicit-`unrestricted` rule from §2.2 in code form. Every dispatcher path that constructs a `ToolContext` for an activity reaches this line, so omitting `fsProfile:` always means "unrestricted within whatever the policy says about unrestricted" — never "no policy at all."

`crates/orbit-core/src/runtime/pipeline.rs` runs a different fallback for legacy pipeline contexts. If a `ToolContext` arrives without `fs_profile`, the pipeline calls `read_activity_fs_profile_from_env()`, which returns the value of `ORBIT_ACTIVITY_FS_PROFILE` or `None` if the variable is unset or empty. This is **not** equivalent to the v2 host's behavior: when the env var is unset, `ctx.fs_profile` stays `None`, and `enforce_fs_policy` returns `Ok(None)` so fs work proceeds unguarded. The pipeline path is documented here as a real gap rather than as a fallback that mirrors `unrestricted`. See [§9](#9-concerns--honest-limitations).

---

## 7. Sandbox / Exec Primitives

`orbit-exec` is the process-spawn layer. The public surface is in `crates/orbit-exec/src/lib.rs`:

- `ExecRequest { program, args, current_dir, timeout_ms, stdin_mode, environment_mode, debug }` — the request shape.
- `EnvironmentMode::Inherit` (default) or `ClearAndSet(Vec<(String, String)>)`. The `Debug` impl redacts values whose keys match `is_sensitive_env_name`, so debug-printed `ExecRequest` does not leak provider keys.
- `StdinMode::Inherit` / `Null` / `Bytes(Vec<u8>)`.
- `Sandbox` trait (`crates/orbit-exec/src/sandbox.rs`) with one method `validate(req) -> Result<()>`. The default `NoSandbox` always returns `Ok`.
- `run_process(req, sandbox) -> ExecutionResult`.

`run_process` orders the work as: `sandbox.validate` first, then `process::spawn` (`crates/orbit-exec/src/process.rs`), then `supervision::wait_with_optional_timeout`. Spawn applies `EnvironmentMode` (clear + set when requested), wires stdout/stderr to piped descriptors, and on Unix calls `command.process_group(0)` so the child becomes its own process-group leader. That single setting is what allows the cleanup layer to kill orphan subprocesses later.

`ExecutionResult { success, stdout, stderr, exit_code, duration_ms, output }` is the result shape (defined in `orbit-common`). The runner converts captured bytes via `String::from_utf8_lossy`, so non-UTF-8 output is preserved as replacement characters rather than failing the call.

The `Sandbox` trait is the seam where future kernel-level isolation (bubblewrap, sandbox-exec, container) would attach. Today the only impl is `NoSandbox`. Tools that do not pre-validate their work get whatever isolation the host OS provides for the orbit process itself.

---

## 8. Process Supervision

`crates/orbit-exec/src/supervision/wait.rs::wait_with_optional_timeout` orchestrates child lifetime. The contract:

1. Spawn background threads to drain stdout and stderr (`spawn_stdout_drain`, `spawn_stderr_drain` from `tee.rs`). These prevent the child from blocking on a full pipe buffer, which is the canonical "stuck child" failure mode for naive subprocess code.
2. If `StdinMode::Bytes` is set, spawn an additional stdin-writer thread (`spawn_stdin_write`).
3. On Unix, install a `SignalHandlerGuard` that captures SIGINT and SIGTERM via `libc::sigaction`. The handler writes a single byte to a non-blocking pipe; the wait loop reads from that pipe to learn that a parent-side signal arrived.
4. Loop with `WAIT_POLL_INTERVAL = 100ms`, calling `child.wait_timeout(slice)`. The slice is the smaller of the poll interval and the remaining deadline.
5. When the child exits cleanly, call `kill_process_group(child.id())` to reap any orphan subprocesses still holding the pipe write ends, then join the reader threads.
6. When a parent-side signal arrives, call `terminate_process_group(child, signal, poll_interval)` and report `exit_code = Some(128 + signal)` with a `process interrupted by signal SIG…` line appended to stderr.
7. When the deadline expires, call `terminate_process_group(child, SIGTERM, poll_interval)` and append `process timed out` to stderr.

`crates/orbit-exec/src/supervision/cleanup.rs` is the termination layer. The escalation policy:

1. Send `SIGTERM` (or the supplied signal) to the entire process group via `killpg`.
2. Poll `process_group_is_alive(pid)` for up to `TERMINATION_GRACE_PERIOD = 5 seconds`.
3. If the group is gone, return success.
4. Otherwise send `SIGKILL` to the group, then call `child.kill()` and `child.wait()` to reap.

`process_group_is_alive` uses `killpg(pid, 0)` and treats `ESRCH` as "all gone." Any other errno is treated as "still alive" so we err on the side of escalating to SIGKILL.

`crates/orbit-exec/src/supervision/signal.rs::SignalHandlerGuard` is an RAII guard. Install acquires a global `Mutex` (so only one wait loop installs handlers at a time), creates a non-blocking pipe, swaps in the orbit-exec handler for SIGINT and SIGTERM, and remembers the previous `sigaction` structs. Drop reverses every step in order — restore previous handlers, close the pipe, release the mutex. The handler itself is `unsafe extern "C"` and only does an atomic load and a single-byte `write`, both of which are async-signal-safe.

Non-Unix builds use a fallback `terminate_process_group` that just calls `child.kill().ok(); child.wait().ok();` — process-group semantics do not apply on Windows, so orphan reaping is best-effort.

---

## 9. Concerns & Honest Limitations

1. **`NoSandbox` is the only Sandbox impl.** The trait has shipped, but no kernel-level or container-level isolation is wired in. A malicious or buggy tool that bypasses the policy layer can still touch the host filesystem. The policy/tool layer is the load-bearing safety surface today.
2. **Policy enforcement is HTTP-backend-only.** `backend: cli` activities spawn an external CLI agent that owns its own filesystem behavior; `enforce_fs_policy` never runs for them. `fsProfile:` is recorded on the activity and surfaces in audit, but it does not gate fs access during a CLI-backed run. The harness-delegated allowlist event documents the gap rather than closing it. Closing this requires either ending CLI-backed dispatch, wrapping CLI runtimes in an OS-level sandbox, or routing CLI fs through a trapping shim.
3. **Pipeline env-fallback can leave `fs_profile = None`.** `crates/orbit-core/src/runtime/pipeline.rs` reads `ORBIT_ACTIVITY_FS_PROFILE` to fill a missing `fs_profile`. If the env var is unset, the `ToolContext` keeps `fs_profile: None`, `enforce_fs_policy` returns `Ok(None)`, and fs work proceeds unguarded. This diverges from the v2 host's `tool_context_for_activity`, which always materializes `unrestricted`. Callers that construct contexts outside the v2 dispatcher must set `fs_profile` explicitly or accept the unguarded path.
4. **Policy enforcement is not centralized within HTTP-backed runs either.** `enforce_fs_policy` is called from `orbit-tools::builtin::fs`, but a future tool (or a non-builtin tool) that performs fs work without going through that helper would not be guarded. There is no fs-trait-level policy interception.
5. **Exec has no policy hook.** `proc.spawn` and similar shell-invoking tools do not consult the `PolicyEngine`. Program allowlists are recorded at the activity layer (`activity.rs` notes that `spec.allowed_programs` is consulted by the proc.spawn tool), but those allowlists are not part of the `PolicyDef` and do not flow through `effective_profile`.
4. **Symlink semantics are not specified.** `workspace_relative_path` canonicalizes via `Path::canonicalize`, which follows symlinks and rejects out-of-workspace targets. The deny semantics for a symlink that points outside the workspace are "denied because the resolved path is not workspace-relative," but this is not currently spelled out as an invariant.
5. **Glob translator is narrow.** It supports `*`, `**`, `?`, and `<prefix>/**`. It does not support character classes (`[abc]`), brace expansion (`{a,b}`), or POSIX bracket expressions. New profile shapes that need richer matching will hit translator gaps before they hit the evaluator.
6. **`PolicyDecision` and `FsPolicyEvaluation` are parallel surfaces.** The fs path uses the evaluation struct; the broader RBAC plumbing uses the simple Allow/Deny enum. There is no current bridge that converts one to the other, which means future non-fs policy evaluators have to pick a return shape rather than reuse one.
7. **Empty-rule-set semantics are conservative but non-obvious.** A profile with no `read` rules denies every read with `matched_rule = "[]"`. This is the safe default, but a user who declares a profile with only `denyRead` rules and no positive rules will see "[]" denials, not the "matched the deny rule" denial they may expect.
8. **Signal handler installation is process-global.** `SignalHandlerGuard` uses a global `Mutex` to serialize installs, so two concurrent `run_process` calls in the same process must take turns installing handlers. This is currently fine because v2 dispatch is single-threaded per process, but it is a real constraint if exec ever moves into a worker pool.
9. **Workspace canonicalization can deny legitimate paths.** If `workspace_root.canonicalize()` fails (e.g., the directory was just deleted), the helper falls back to the non-canonical root, which can cause `strip_prefix` to fail and surface as a `PolicyDenied("path is outside workspace")` error rather than a clearer "workspace missing" error.

---

## Task References

- **[T20260416-0728]** — Align policy contract with runtime enforcement; established v2 schema and effective-profile resolution.
- **[T20260417-0550]** — Decompose `orbit-exec` supervision modules.
- **[T20260417-0557]** — Harden Orbit path boundaries and dependency advisories.
- **[T20260417-0558-4]** / **[T20260417-0558-5]** — Harden `orbit-exec` supervision (signal-pipe handler and process-group reaping).
- **[T20260419-0503]** — Enforce `fsProfiles` across runtime and CLI; introduced the `tool_context_for_activity` materialization.
- **[T20260328-221810]** — Agent subprocess termination on Ctrl+C / job-run cancel; predecessor of the current signal-pipe design.
- **[T20260426-0605]** — Auditability design folder cross-linked from §5.
- **[T20260426-0622]** — Add this policy & sandboxing design folder and document the current contract.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
