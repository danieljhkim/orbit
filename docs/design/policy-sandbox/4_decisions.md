# Policy & Sandboxing — Decisions

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-16

This is the append-only ADR log for Policy & Sandboxing. Entries are ordered by ADR number. New entries follow the template in [../CONVENTIONS.md](../CONVENTIONS.md) and cite the task that made the decision real.

---

## ADR-001 — Dedicated policy & sandboxing design ownership

**Status:** Accepted · 2026-04 · [T20260426-0622]

**Context.** Policy and sandboxing semantics were spread across `orbit-policy`, `orbit-exec`, the `PolicyDef` schema in `orbit-common`, the activity dispatcher, and the v2 host. There was no canonical place to record invariants, the `unrestricted` fallback, or the supervision contract.

**Decision.** Create `docs/design/policy-sandbox/` as the canonical design folder, with claude as owner. Auditability owns the recording of denials; this folder owns the *semantics* of allow/deny and the *contract* for how spawned processes are supervised.

**Consequences.**
- Policy and sandboxing decisions now have one ADR log, one glossary, and a feature-owned spec to cite.
- Cost: this folder cross-links into auditability and activity-job, so when those folders change their cross-references must be kept in sync rather than this folder absorbing them.

## ADR-002 — Policy schema is v2-only with named profiles plus global denies

**Status:** Accepted · 2026-04 · [T20260416-0728]

**Context.** An earlier policy schema (v1) used a different shape for allow/deny rules. Supporting both shapes in the runtime caused interpretation drift between the loader, the merger, and the evaluator.

**Decision.** Reject `schemaVersion: 1` at load time with an explicit migration message. v2 declares `denyRead`, `denyModify`, and `fsProfiles` and is the only accepted shape. Workspace policies override globals by profile name; global denies accumulate.

**Consequences.**
- Schema parsing has one supported branch, and profile authoring is uniformly `{ read, modify }` with global denies.
- Cost: existing v1 policy files require a manual migration; there is no automatic upgrader.

## ADR-003 — Implicit `unrestricted` profile materializes when an activity omits `fsProfile:`

**Status:** Accepted · 2026-04 · [T20260419-0503]

**Context.** Activities can omit `fsProfile:`. A naive design would either reject the activity at load or run it without policy enforcement. Both are wrong: rejection breaks the common case, and unguarded execution means audit blindness.

**Decision.** When an activity omits `fsProfile:`, the v2 host substitutes the constant `UNRESTRICTED_FS_PROFILE` ("unrestricted") at `tool_context_for_activity`. If the policy does not define a profile of that name, the resolver synthesizes `read: ["./**"]` and `modify: ["./**"]`. Global `denyRead` / `denyModify` rules still apply because they are injected after profile resolution.

**Consequences.**
- "Unrestricted" remains auditable and narrowed by global denies, while policy authors can shadow it with a real profile.
- Cost: the word "unrestricted" carries different meaning depending on whether the policy defines a profile of that name, which is a learnable but real source of confusion.

## ADR-004 — Deny rules inject as negated profile rules with last-match-wins evaluation

**Status:** Accepted · 2026-04 · [T20260416-0728]

**Context.** A separate "deny pass" before profile evaluation is the obvious shape, but it makes precedence ambiguous when a profile rule and a deny rule both match. Multiple Orbit features (workspace overrides, profile narrowing, denyModify-also-implies-denyRead-for-modify validation) need a single evaluation order.

**Decision.** `effective_profile` appends every entry of `denyRead` to the profile's `read` list as `!<rule>` and every entry of `denyModify` to the profile's `modify` list as `!<rule>`. `check_path` walks the resolved list in order and the **last match wins**. There is no separate deny pass.

**Consequences.**
- Profile rules and deny rules are evaluated in one deterministic pass; appended denies win over earlier positive matches.
- Cost: a profile author cannot re-allow a globally denied path by ordering, which is the intended safety property but surprises authors who expect a simple allowlist with overrides.

## ADR-005 — Modify rules must be covered by a read rule in the same profile

**Status:** Accepted · 2026-04 · [T20260416-0728]

**Context.** A profile that grants `modify: ["./build/**"]` without granting `read: ["./build/**"]` is technically valid but produces a confusing operational story: a tool may be allowed to write a file it cannot read, breaking the standard read-modify-write pattern.

**Decision.** `PolicyDef::validate` rejects any profile whose positive `modify` rule is not covered by a positive `read` rule in the same profile. "Covered" is checked structurally (`rule_covers_path_rule`): exact match, `**`, or a `<prefix>/**` rule that prefixes the modify rule.

**Consequences.**
- Modify rules require corresponding read coverage, so read-modify-write audit stories stay consistent.
- Cost: profile authors who *only* want to allow append-style writes cannot express that without granting a read rule. There is no "write-only" profile shape today.

## ADR-006 — Tool layer is the policy enforcement point for HTTP-backed activities

**Status:** Accepted · 2026-04 · [T20260419-0503]

**Context.** Policy enforcement could plausibly live at the syscall layer, the fs trait layer, the tool layer, or the activity layer. Each placement has different trust and coverage tradeoffs.

**Decision.** Enforcement lives in `orbit-tools::builtin::fs::enforce_fs_policy`. Every fs builtin calls it before the underlying read or modify, and emits `FsCallEvent` through `FsAuditLogger`. The `Sandbox` trait in `orbit-exec` does not consult the policy engine; exec is supervised but not policy-gated. This applies only to `backend: http` activities — `backend: cli` runs spawn an external CLI agent and emit a `tool_allowlist.harness_delegated` event in lieu of enforcement.

**Consequences.**
- HTTP-backed fs decisions have one auditable helper, but tool authors must route work through it.
- Cost: CLI-backed activities still bypass this helper, and HTTP tools that skip it are also unguarded. Current macOS executors can narrow CLI filesystem writes with `sandbox-exec`, but closing the general gap likely requires a `PolicyAwareFs` trait, broader OS sandboxes, or both.

## ADR-007 — Children spawn as process-group leaders so orphan subprocesses are reapable

**Status:** Accepted · 2026-04 · [T20260417-0558-4], [T20260328-221810]

**Context.** Naive subprocess code on Unix leaves orphan grandchildren holding open pipe write ends, which causes the parent's `wait_with_output` to hang when the orphan never exits. Earlier versions of orbit-exec hit this exact failure when an agent's tool spawned long-lived helpers.

**Decision.** On Unix, every spawned child calls `command.process_group(0)` so the child becomes a process-group leader (PGID = PID). The supervision layer kills the entire group via `killpg` when the child exits, when the parent receives SIGINT/SIGTERM, or when the deadline expires.

**Consequences.**
- Orphan subprocesses are reaped, and signal handling can target the whole tree with one syscall.
- Cost: tools that intentionally fork detached helpers (e.g., long-running daemons) cannot do so under orbit-exec without explicitly creating their own process group inside the child.

## ADR-008 — SIGTERM with 5-second grace, then SIGKILL for the whole group

**Status:** Accepted · 2026-04 · [T20260417-0558-4]

**Context.** A timed-out or interrupted child needs a chance to flush state before being killed, but the supervisor cannot wait indefinitely. The escalation policy needs a single, predictable shape.

**Decision.** `terminate_process_group` sends `SIGTERM` (or the supplied signal) to the group, polls `process_group_is_alive` for `TERMINATION_GRACE_PERIOD = 5 seconds`, and on expiry sends `SIGKILL` to the group plus a direct `child.kill()`/`child.wait()`. stderr is annotated with `process timed out` (deadline path) or `process interrupted by signal SIG…` (parent-signal path).

**Consequences.**
- Termination is deterministic, and annotated stderr distinguishes timeout, signal, and clean-exit paths.
- Cost: the 5-second constant is global. Activities that need a longer drain (database flush, large I/O cleanup) cannot extend it without code changes.

## ADR-009 — Signal handler installation is process-global and serialized

**Status:** Accepted · 2026-04 · [T20260417-0558-5]

**Context.** Installing parent-side SIGINT/SIGTERM handlers is a process-global operation. Two concurrent `run_process` calls cannot install independent handlers without races, and a panicking call must restore the prior handler so the orbit process itself remains interruptible.

**Decision.** `SignalHandlerGuard::install` acquires a `Mutex` from a `OnceLock`, creates a non-blocking pipe, calls `libc::sigaction` for SIGINT and SIGTERM, and stores the previous `sigaction` structs. Drop reverses the steps: restore previous handlers, close the pipe, release the mutex. The handler itself is async-signal-safe (atomic load + 1-byte `write`).

**Consequences.**
- Concurrent `run_process` calls serialize handler install/drop, and panics still restore prior handlers via Drop.
- Cost: contention on the global mutex limits exec parallelism in a single process. Named as an open question in [3_vision.md §1.11](./3_vision.md#1-open-questions).

## ADR-010 — `NoSandbox` is the default `Sandbox` impl; real isolation is deferred

**Status:** Accepted · 2026-04 · [T20260417-0550]

**Context.** The `Sandbox` trait is the seam where kernel-level or container-level isolation would attach to `orbit-exec`. The trait shipped with the supervision rework, but no real impl is registered.

**Decision.** Ship `NoSandbox` as the default and only implementation. Defer kernel-level isolation (bubblewrap, sandbox-exec, container, seccomp) until policy enforcement at the tool layer is judged insufficient and the platform-coverage cost is understood. The trait surface is stable so a future impl can attach without changing the runner.

**Consequences.**
- The trait surface is stable for future isolation, while today's generic runner stays explicit about relying on tool-layer policy.
- Cost: a tool that performs fs work without `enforce_fs_policy` (or a future non-builtin tool) has no exec-level isolation backstop. This is the structural reason §1.1 of [3_vision.md](./3_vision.md) lists real sandboxing as the top open question.

## ADR-011 — `sandbox-exec` wraps cli-backend agent invocations on macOS

**Status:** Accepted · 2026-04 · [T20260427-51]

**Context.** ADR-006 left CLI backends outside Orbit's tool-layer enforcement: the harness emits `tool_allowlist.harness_delegated`, but Claude/Codex/Gemini built-in tools run with the orbit process's filesystem rights. Provider-native sandboxes were inconsistent (`codex --sandbox`, `gemini -s`, no Claude equivalent), leaving `fsProfile` unenforced for some CLI runs.

**Decision.** Add `orbit-exec::macos_sandbox` as the declarative seam: compile a `ResolvedFsProfile` to SBPL and wrap Claude, Codex, and Gemini invocations with `sandbox-exec -f <profile>` when executor YAML declares `spec.sandbox: macos-sandbox-exec`. When Orbit owns the outer sandbox, neutralize provider-native sandbox flags so there is one filesystem authority. Resolve descriptors in `V2RuntimeHost::resolve_executor_sandbox` and compile SBPL in orbit-engine near the spawn site.

**Consequences.**
- All three providers share `FsProfile` compiled to SBPL as the macOS filesystem authority, giving Claude OS-enforced narrowing too.
- `allow_fallback` can degrade gracefully, but the safe default is fail-closed; Linux, Docker, network restriction, and activity-level overrides stay out of scope for v1.
- Cost: SBPL writes are static text; complex `denyRead` / `denyModify` rule combinations don't always translate cleanly. Simple subtree denials use `subpath`; non-subpath deny globs use SBPL `regex` to avoid over-denying the containing directory. Activities that need precise allow-side glob semantics under sandbox should declare profiles with explicit subpath roots.

## ADR-012 — Codex state and side roots are narrow sandbox write allowances

**Status:** Accepted · 2026-04 · [T20260428-10]

**Context.** Codex-backed `agent_implement` reached startup under `sandbox-exec` but failed with `Operation not permitted`: the profile allowed worktree, temp/cache, and `$HOME/.orbit` writes but not Codex state. After that, workflow state still failed because policy denied workspace `.orbit/**` after Orbit passed the same root via Codex `--add-dir`, and `**/*.env` over-denied when compiled as a containing-directory `subpath`.

**Decision.** Keep `sandbox-exec` authoritative and add narrow Codex allowances: `$CODEX_HOME` or `$HOME/.codex`, plus Codex side-write roots from runtime provider config appended after policy-derived denials. Compile non-subpath deny globs such as `**/*.env` as SBPL `regex` clauses. Do not grant broad `$HOME` writes or disable the outer sandbox.

**Consequences.**
- Codex-backed `backend: cli` runs can initialize under the macOS sandbox while project writes stay constrained by the resolved `fsProfile`.
- `CODEX_HOME` relocates state, and inherited Orbit subprocesses can persist workflow state through the same side roots Codex receives.
- Cost: the Codex state directory and provider side roots are trusted writable state outside ordinary project-content policy, similar to the existing `$HOME/.orbit` allowance for inherited Orbit subprocesses.

## ADR-013 — Per-provider state-dir allowances are emitted unconditionally for every supported CLI

**Status:** Accepted · 2026-04 · [T20260428-14]

**Context.** ADR-012 unblocked Codex state writes, but Claude writes startup state under `$HOME/.claude` or `$CLAUDE_CONFIG_DIR`, Gemini writes under `$HOME/.gemini`, and Grok writes under `$HOME/.grok`. SBPL compilation receives `ResolvedFsProfile` plus host env, not the active provider, so provider-conditional allow clauses would require new plumbing.

**Decision.** Emit state-dir allows for all supported CLI providers on every macOS sandbox profile: `$CODEX_HOME` / `$HOME/.codex`, `$CLAUDE_CONFIG_DIR` / `$HOME/.claude`, `$HOME/.gemini`, and `$HOME/.grok`. Keep `append_provider_side_write_roots` Codex-only because Claude, Gemini, and Grok have no `--add-dir` equivalent; document that a future provider with such a surface should generalize the branch.

**Consequences.**
- Claude, Gemini, and Grok reach past CLI startup under `macos-sandbox-exec` with the same state-dir defense story as Codex.
- Emitting all four narrow state-dir allowances avoids provider plumbing; Codex side roots remain a separate branch until another provider ships an equivalent surface.
- Cost: every macOS sandbox profile carries four state-dir allow clauses regardless of which provider runs. If a future provider's state dir overlaps with another sensitive root, this design needs revisiting.

## ADR-014 — Claude state surface includes `$HOME/.claude.json` siblings, not just `$HOME/.claude/`

**Status:** Accepted · 2026-05 · [T20260508-13]

**Context.** ADR-013 modeled Claude's state surface as the `$HOME/.claude/` directory (or `$CLAUDE_CONFIG_DIR` when set) and emitted a single `(allow file-write* (subpath ...))` clause per provider state dir. In practice, Claude Code persists its main settings to `$HOME/.claude.json` — a sibling *file* at the home root, with `.lock` and atomic-write `.tmp.<pid>.<ms_ts>` companions. SBPL `subpath` only matches the named directory and everything strictly below, so `.claude.json` (a sibling, not a child) was denied at the kernel. Symptom: every Claude invocation under `macos-sandbox-exec` lost the ability to update its state, and tool calls that wait on the state-file lock hung silently. Codex/Gemini were unaffected because all of their state lives under their state directories.

The override case is clean: when `CLAUDE_CONFIG_DIR` is set, Claude writes `<override>/.claude.json` and its siblings inside the override directory, already covered by the existing `(subpath "$CLAUDE_CONFIG_DIR")` clause.

**Decision.** When the SBPL profile is compiled with `CLAUDE_CONFIG_DIR` unset and `HOME` resolved, additionally emit:

- `(allow file-write* (literal "$HOME/.claude.json"))`
- `(allow file-write* (literal "$HOME/.claude.json.lock"))`
- `(allow file-write* (regex "^$HOME/\.claude\.json\.tmp\.[0-9]+\.[0-9]+$"))`

Use `literal` for the canonical and lock files (predictable names) and `regex` for the tmp pattern. The home prefix in the regex is escaped with the existing `push_regex_escaped` helper so symlink-free home paths containing regex meta characters do not widen the allow.

**Consequences.**
- Claude under `macos-sandbox-exec` can persist settings and acquire its lockfile; tool calls that depend on a freshly-updated state file no longer hang.
- The `CLAUDE_CONFIG_DIR` branch is unchanged — the existing subpath clause already covers the JSON file inside the override.
- Cost: three additional clauses on every macOS sandbox profile when `HOME` resolves and `CLAUDE_CONFIG_DIR` is unset. Symmetric to the ADR-013 trade-off; provider plumbing is avoided.
- This ADR amends ADR-013 rather than replacing it: the per-provider state-dir clauses still emit unconditionally; the new clauses are scoped to the HOME-fallback branch only.

## ADR-015 — macOS sandbox wrapper resolves from trusted absolute locations

**Status:** Accepted · 2026-05 · [T20260509-30]

**Context.** The macOS CLI wrapper previously spawned `sandbox-exec` by bare name and checked availability by walking `PATH`. A writable or config-influenced `PATH` could point Orbit at an attacker-controlled wrapper while Orbit still believed kernel sandbox enforcement was active.

**Decision.** Resolve the wrapper only from trusted absolute locations, currently `/usr/bin/sandbox-exec`, and use the same trusted resolver for availability checks, audit argv, and process spawn. Missing trusted binaries fail closed unless the executor explicitly allows fallback, and the error names the trusted location that was probed.

**Consequences.**
- Fake `sandbox-exec` binaries earlier on `PATH` are ignored, so the sandbox boundary no longer depends on inherited environment ordering.
- Availability messages describe the trusted absolute location instead of implying arbitrary `PATH` lookup.
- Cost: the implementation is intentionally macOS-location-specific; if Apple moves or removes the binary, Orbit must update the trusted location list or add a new backend rather than silently accepting a user-supplied replacement.

---

## Task References

- **[T20260328-221810]** — Subprocess termination on Ctrl+C / job cancel; predecessor of the current process-group design.
- **[T20260416-0728]** — Aligned the policy contract with runtime enforcement; v2 schema and effective-profile resolution land here.
- **[T20260417-0550]** — Decomposed `orbit-exec` supervision modules.
- **[T20260417-0558-4]** / **[T20260417-0558-5]** — Hardened `orbit-exec` supervision (process-group reaping, signal-pipe handler).
- **[T20260419-0503]** — Enforced `fsProfiles` across runtime and CLI; introduced `tool_context_for_activity`.
- **[T20260426-0622]** — Add this design folder and record the initial ADR set.
- **[T20260427-51]** — Wrap cli-backend agent invocations in `sandbox-exec` on macOS with inner-flag neutralization for codex/gemini.
- **[T20260428-10]** — Allow Codex CLI state writes under the macOS sandbox.
- **[T20260428-14]** — Extend the macOS sandbox state-dir allowance to Claude and Gemini, and document why side-write roots remain Codex-only.
- **[T20260430-23]** — Shorten the policy sandbox design docs while preserving the shipped contract and ADR history.
- **[T20260508-13]** — Add `$HOME/.claude.json{,.lock,.tmp.<pid>.<ms_ts>}` sibling allows to the macOS sandbox profile so Claude can persist its main settings file.
- **[T20260509-30]** — Resolve `sandbox-exec` from trusted absolute locations rather than inherited `PATH`.
- **[ORB-00048]** — Extend the unconditional provider state-dir allowance set to include Grok's `$HOME/.grok` state directory while hardening fourth-family scoreboards and analytics.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
