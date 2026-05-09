# Policy & Sandboxing — Design

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-09 (T20260509-7, T20260509-28, T20260509-30)

This document describes Orbit's shipped policy and sandboxing implementation: v2 `PolicyDef`, profile resolution, last-match-wins path evaluation, HTTP-tool enforcement, activity/job `fsProfile` binding, macOS CLI sandbox wrapping, and `orbit-exec` supervision. See [1_overview.md](./1_overview.md) for purpose and [3_vision.md](./3_vision.md) for forward-looking gaps.

---

## 1. Policy Schema

`PolicyDef` in `crates/orbit-common/src/types/policy_def.rs` is v2-only. `crates/orbit-common/src/types/resource.rs` rejects schema v1 with a migration message that names `spec.denyRead`, `spec.denyModify`, and `spec.fsProfiles`.

A valid policy declares `name`, optional `description`, global `denyRead` / `denyModify`, and `fsProfiles` mapping names to `FsProfile { read, modify }`. The policy name must also pass the centralized resource-name validator in `crates/orbit-common/src/types/resource.rs`: it is a non-empty single file stem, not a hidden dot name, and contains no separators, traversal markers, drive-prefix characters, extension dots, or control characters ([T20260509-28]). File-backed stores validate before constructing `<name>.yaml` paths.

`PolicyDef::validate` enforces:

1. The policy name is a safe resource file stem.
2. Every profile name is non-empty.
3. Every positive `modify` rule is covered by a positive `read` rule in the same profile.
4. Profile rules do not exactly duplicate global deny entries.

`PolicyDef::merged(global, workspace)` lets workspace `fsProfiles` overwrite globals by name while global denies accumulate. The merged policy is revalidated.

---

## 2. Profile Resolution

`PolicyDef::effective_profile(profile_name)` returns a `ResolvedFsProfile { name, read, modify }` after applying three transformations:

1. **Lookup.** Use the named profile. If the missing name is `unrestricted`, synthesize `read: ["./**"]` and `modify: ["./**"]`; other missing profiles return `OrbitError::InvalidInput`.
2. **Normalization.** Trim, convert backslashes, strip leading `./`, reject absolute, `~`, and parent-traversal rules, then compile the narrow glob syntax to regex.
3. **Deny injection.** Append `denyRead` to `read` and `denyModify` to `modify` as negated rules (`!<rule>`), so global denies participate in the same ordered list.

The implicit `unrestricted` profile appears only when an activity omitted `fsProfile:` and the policy did not define `unrestricted`. A real profile with that name shadows the fallback.

---

## 3. Path Evaluation

`PolicyDef::check_path(profile, op, path)` returns an `FsCheckResult { allowed, matched_rule }`. The algorithm:

1. Resolve the profile (via §2).
2. Pick the rule list by operation (`read` or `modify`).
3. If the list is empty, deny with `matched_rule = "[]"`.
4. Walk rules in order and record the most recent match against the normalized workspace-relative path. Later matches override earlier ones.
5. Use the last match's negation flag. If no rule matched but a positive rule exists, deny with `<no matching rule>`; if only negated rules exist, deny with `[]`.

Path normalization (`normalize_path`) trims, flips slashes, strips `./` prefixes, and rejects absolute paths, `~`-anchored paths, and parent-directory traversal anywhere in the component list ([T20260509-27]). Tool callers are expected to canonicalize first and then express the path workspace-relative — `crates/orbit-tools/src/builtin/fs/mod.rs::workspace_relative_path` handles that on the call site.

The glob translator supports `*`, `**`, `?`, and `<prefix>/**`. It is intentionally narrower than POSIX glob syntax.

---

## 4. PolicyEngine Facade

`crates/orbit-policy/src/lib.rs` re-exports `PolicyEngine`, `FsPolicyEvaluation`, and `PolicyDecision`. `PolicyEngine` wraps a validated `PolicyDef` and exposes:

```
PolicyEngine::check(profile, operation, path) -> FsPolicyEvaluation
```

`FsPolicyEvaluation` carries `{ profile, operation, path, allowed, matched_rule }`. `evaluator.rs` currently passes through to `PolicyDef::check_path`; the indirection leaves room for caching or layered evaluators later.

`PolicyDecision` (`crates/orbit-common/src/types/policy_decision.rs`) is a separate `Allow | Deny { reason }` enum for broader policy/RBAC callers. `PolicyEngine::check` does not produce it; fs callers use `FsPolicyEvaluation`.

---

## 5. Tool-Layer Enforcement

`crates/orbit-tools/src/builtin/fs/mod.rs::enforce_fs_policy` is the only place fs operations consult the policy engine today. It reads `ctx.fs_profile` and `ctx.policy_engine`; if either is missing, it returns `Ok(None)` so fs work proceeds unguarded. That path is for unit tests / no-policy contexts, not the real v2 host path. Otherwise the helper converts the canonical path to workspace-relative form, calls `policy_engine.check`, emits a request or denied `FsCallEvent`, and returns either an `FsPolicyAllowance { profile, op, path, matched_rule }` or `OrbitError::PolicyDenied`.

The audit emission goes through `ctx.fs_audit: Option<Arc<dyn FsAuditLogger>>` (`crates/orbit-tools/src/lib.rs`). The v2 dispatcher wires this to `v2_fs_audit_logger(audit.clone())`, which converts each `FsCallEvent` into a `V2AuditEvent` filesystem entry. The full audit-channel description belongs to [auditability](../auditability/2_design.md#3-tool-driven-and-runtime-audit-records); this folder owns the *enforcement* contract, not the storage contract.

`FsCallEvent` carries `{ kind, profile, op, path, allowed, matched_rule }`. There is no persisted negation flag; consumers that need to distinguish explicit deny matches from "no rule matched" must compare `matched_rule` with the policy denies. The exec layer does not consult the policy engine, so there is no `proc.spawn` policy gate today.

**Backend scope.** This enforcement fires only under `backend: http` when a builtin fs tool runs. `backend: cli` spawns Claude Code, Codex CLI, Gemini, or another harness via `cli_runner.rs`, emits `tool_allowlist.harness_delegated`, and trusts that harness for tool allowlists. On macOS, executors declaring `sandbox: macos-sandbox-exec` also get the OS-level wrapper in §7, so `fsProfile:` can still narrow CLI filesystem writes.

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

This is the implicit-`unrestricted` rule from §2.2 in code form. Every v2 dispatcher path that constructs a `ToolContext` reaches this line, so omitting `fsProfile:` means "unrestricted within policy," not "no policy."

Legacy pipeline contexts are different. `crates/orbit-core/src/runtime/pipeline.rs` fills a missing profile from `ORBIT_ACTIVITY_FS_PROFILE`; if the variable is unset, `ctx.fs_profile` stays `None` and `enforce_fs_policy` returns `Ok(None)`. That unguarded path is a real gap, not another spelling of `unrestricted` (see §9).

---

## 7. Sandbox / Exec Primitives

`orbit-exec` is the process-spawn layer. The public surface is in `crates/orbit-exec/src/lib.rs`:

- `ExecRequest { program, args, current_dir, timeout_ms, stdin_mode, environment_mode, debug }`.
- `EnvironmentMode::Inherit` or `ClearAndSet(Vec<(String, String)>)`; debug output redacts sensitive env values.
- `StdinMode::Inherit` / `Null` / `Bytes(Vec<u8>)`.
- `Sandbox::validate(req) -> Result<()>`; the default `NoSandbox` always returns `Ok`.
- `run_process(req, sandbox) -> ExecutionResult`.

`run_process` calls `sandbox.validate`, then `process::spawn`, then `supervision::wait_with_optional_timeout`. Spawn applies the requested environment, pipes stdout/stderr, and on Unix calls `command.process_group(0)` so cleanup can kill orphan subprocesses.

`ExecutionResult { success, stdout, stderr, exit_code, duration_ms, output }` is defined in `orbit-common`. Captured bytes use `String::from_utf8_lossy`, so non-UTF-8 output becomes replacement characters instead of failing the call.

The `Sandbox` trait remains the seam for generic `run_process` callers, but CLI-backed `agent_loop` invocations use a separate executor wrapper when the executor declares `sandbox: macos-sandbox-exec` ([T20260427-51]). The v2 host resolves the activity `fsProfile`; the engine converts workspace-relative rules to absolute roots and compiles SBPL before spawning the provider CLI.

The macOS wrapper resolves `sandbox-exec` from trusted absolute locations only, currently `/usr/bin/sandbox-exec`; it does not consult `PATH` for either availability checks or process spawn. If the trusted binary is missing, the runner fails closed unless the executor declares `allow_fallback: true`, and the error names the trusted location that was probed ([T20260509-30]).

The compiled macOS profile denies by default, allows broad reads required by agent CLIs and system libraries, allows process/signal/ipc/network/sysctl/iokit operations, and allows writes to:

- scratch/cache roots (`/tmp`, `/private/tmp`, `/private/var/folders`, `/dev`, `$HOME/Library/Caches`)
- `$HOME/.orbit` for inherited Orbit subprocess audit/state
- provider state dirs: Codex (`$CODEX_HOME` or `$HOME/.codex`), Claude (`$CLAUDE_CONFIG_DIR` or `$HOME/.claude`), and Gemini (`$HOME/.gemini`)
- positive `modify` roots from the resolved profile
- Codex side-write roots from runtime provider config, appended after policy denies so workflow state remains writable under the outer sandbox

Negated `read` / `modify` rules become explicit SBPL denies after ordinary profile allows to preserve last-match-wins. Simple path and `/**` subtree denials compile to `subpath`; non-subpath globs such as `**/*.env` compile to `regex`. Host-owned provider side roots are the exception because the provider CLI and inherited Orbit subprocesses must write workflow state.

---

## 8. Process Supervision

`crates/orbit-exec/src/supervision/wait.rs::wait_with_optional_timeout` drains stdout/stderr in background threads, writes stdin bytes when requested, installs Unix SIGINT/SIGTERM handling, and polls `child.wait_timeout` every `WAIT_POLL_INTERVAL = 100ms`. Clean exits still call `kill_process_group(child.id())` to reap orphans. Parent signals terminate the group and report `exit_code = Some(128 + signal)` with annotated stderr; deadlines terminate with SIGTERM and append `process timed out`.

`crates/orbit-exec/src/supervision/cleanup.rs` is the termination layer. The escalation policy:

1. Send `SIGTERM` (or the supplied signal) to the entire process group via `killpg`.
2. Poll `process_group_is_alive(pid)` for up to `TERMINATION_GRACE_PERIOD = 5 seconds`.
3. If the group is gone, return success.
4. Otherwise send `SIGKILL` to the group, then call `child.kill()` and `child.wait()` to reap.

`process_group_is_alive` uses `killpg(pid, 0)`, treats `ESRCH` as "all gone," and treats other errno values as "still alive" so cleanup errs toward SIGKILL.

`SignalHandlerGuard` is RAII: install acquires a global `Mutex`, creates a pipe, swaps in handlers, and stores prior `sigaction` structs; Drop restores handlers, closes the pipe, and releases the mutex. The handler performs only an atomic load plus one-byte `write`, both async-signal-safe.

Non-Unix builds use a fallback `terminate_process_group` that just calls `child.kill().ok(); child.wait().ok();` — process-group semantics do not apply on Windows, so orphan reaping is best-effort.

---

## 9. Test surfaces

Risk-weighted regression tests sit beside the implementations they guard
([T20260509-7]):

- `crates/orbit-policy/src/engine.rs#tests` — `PolicyEngine::check` boundary
  semantics: positive read-rule matches return `allowed=true` with the rule
  recorded in `matched_rule`; modify paths outside any positive rule resolve
  to `allowed=false`; global `denyRead` / `denyModify` rules override
  profile-level positive rules under last-match-wins; an unknown profile name
  errors structurally (with the documented `unrestricted` exception); and the
  `matched_rule` field is populated for audit attribution. Traversal inputs
  such as `../secret.txt`, `src/../secret.txt`, and their backslash-normalized
  equivalents are rejected as `OrbitError::InvalidInput` for both read and
  modify checks ([T20260509-27]).
- `crates/orbit-exec/src/macos_sandbox.rs#tests` — trusted wrapper
  resolution ignores `PATH`, including a macOS runtime test that places a fake
  `sandbox-exec` earlier on `PATH` and verifies the fake wrapper is not
  executed ([T20260509-30]). SBPL compilation tests
  cover `denyRead` / `denyModify` clause emission (`subpath` for simple
  rules, `regex` for non-trivial globs) and the deny-after-allow ordering
  required for last-match-wins. macOS-gated runtime tests
  (`compiled_profile_denies_reads_to_negated_read_path` and
  `compiled_profile_for_realistic_agent_loop_profile_allows_repo_writes_denies_dotenv`)
  exercise an `agent_loop`-shaped profile end-to-end against the kernel
  sandbox.
- `crates/orbit-store/src/file/policy_def_store.rs#tests` — policy resource
  name tests reject traversal-shaped names such as `../x` before path
  construction and assert no file is written outside the policy store
  ([T20260509-28]).

Tests skip on non-macOS (and on macOS hosts where `sandbox-exec` cannot
apply) via the existing `cfg(target_os = "macos")` + `sandbox_exec_can_apply()`
gate. SBPL-text assertions paired with each runtime case keep coverage
non-empty on Linux CI.

---

## 10. Concerns & Honest Limitations

1. **OS-level CLI sandboxing is macOS-only.** Linux (`bwrap`), Docker, and other wrappers remain future work; generic `run_process` still defaults to `NoSandbox`.
2. **CLI tool allowlists are delegated.** The macOS wrapper narrows writes, but Orbit still trusts Claude/Codex/Gemini harnesses for declared `tools:`.
3. **Provider state directories are trusted write roots.** `$HOME/.orbit` plus Codex, Claude, and Gemini state dirs are outside the activity workspace and emitted unconditionally.
4. **Codex side-root appends are config-coupled.** If Codex is configured without the workspace-write side roots, inherited Orbit subprocesses can hit `.orbit` write denials.
5. **macOS provenance syscall allowances are private.** `vnguard` and `Sandbox`/67 mirror current Codex startup needs and may require review after OS changes.
6. **Pipeline env fallback can leave `fs_profile = None`.** Legacy contexts without `ORBIT_ACTIVITY_FS_PROFILE` still bypass `enforce_fs_policy`.
7. **HTTP enforcement is helper-based.** A future builtin or non-builtin tool that skips `enforce_fs_policy` is unguarded.
8. **Exec has no policy hook.** `proc.spawn` program allowlists are activity-layer data, not part of `PolicyDef` or `effective_profile`.
9. **Symlink semantics are implicit.** `workspace_relative_path` follows symlinks and rejects out-of-workspace targets, but no spec states that invariant.
10. **Glob syntax is narrow.** Character classes, brace expansion, and POSIX bracket expressions are unsupported.
11. **Policy result shapes are parallel.** `PolicyDecision` and `FsPolicyEvaluation` have no bridge for future non-fs evaluators.
12. **Empty rule sets are safe but opaque.** A profile with only deny rules reports `matched_rule = "[]"`, not the matching deny rule.
13. **Signal handling is process-global.** `SignalHandlerGuard` serializes installs with a global `Mutex`, which constrains future worker-pool exec.
14. **Workspace canonicalization errors collapse to denial.** A missing workspace root can surface as `PolicyDenied("path is outside workspace")` rather than a clearer root-missing error.

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
- **[T20260427-51]** — Wrap cli-backend agent invocations in `sandbox-exec` on macOS.
- **[T20260428-10]** — Allow Codex CLI state writes under the macOS sandbox.
- **[T20260428-14]** — Extend the macOS sandbox state-dir allowance to Claude (`~/.claude` / `$CLAUDE_CONFIG_DIR`) and Gemini (`~/.gemini`), and document why side-write roots remain Codex-only.
- **[T20260430-23]** — Shorten the policy sandbox design docs while preserving the shipped contract and ADR history.
- **[T20260509-7]** — Add `PolicyEngine::check` boundary tests and macOS sandbox `denyRead` / realistic agent-loop profile tests.
- **[T20260509-28]** — Validate policy and executor resource names as safe file stems before file-store path construction.
- **[T20260509-30]** — Resolve `sandbox-exec` from trusted absolute locations and keep availability errors fail-closed and explicit.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
