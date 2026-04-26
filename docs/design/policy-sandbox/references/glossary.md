# Glossary: Policy & Sandboxing

This glossary covers Orbit-specific policy and sandboxing terms only. Generic OS, regex, and security terms are excluded unless Orbit assigns them a specific meaning.

| Term | Meaning |
|------|---------|
| **Allowance** | The `FsPolicyAllowance { profile, op, path, matched_rule }` value built by the tool layer when a path passes `enforce_fs_policy`. Carried through the request → result event pair so the audit shows the same matched rule for both. See [../2_design.md §5](../2_design.md#5-tool-layer-enforcement). |
| **Deny injection** | The mechanism by which `denyRead` / `denyModify` rules become part of a resolved profile: each global deny is appended as `!<rule>` to the profile's `read` or `modify` list before evaluation. See [../2_design.md §2](../2_design.md#2-profile-resolution). |
| **Effective profile** | The `ResolvedFsProfile` returned by `PolicyDef::effective_profile`: profile lookup + normalization + deny injection, with the implicit `unrestricted` fallback applied when the named profile is absent. See [../2_design.md §2](../2_design.md#2-profile-resolution). |
| **FsCallEvent** | The audit event the tool layer emits per fs decision (`Request`, `Result`, or `Denied`) carrying profile, op, path, allowed flag, and matched rule. See [../2_design.md §5](../2_design.md#5-tool-layer-enforcement). |
| **FsPolicyEvaluation** | The `PolicyEngine::check` return shape: `{ profile, operation, path, allowed, matched_rule }`. The fs-specific evaluation result; distinct from the simpler `PolicyDecision` enum used elsewhere. See [../2_design.md §4](../2_design.md#4-policyengine-facade). |
| **Last-match-wins** | Orbit's path evaluation order: walk all rules, the *last* matching rule decides allow vs. deny. Differs from first-match-wins POSIX-style allowlists. See [../2_design.md §3](../2_design.md#3-path-evaluation). |
| **Implicit `unrestricted` profile** | The fallback `FsProfile { read: ["./**"], modify: ["./**"] }` synthesized when an activity omits `fsProfile:` and the policy does not define a profile named `unrestricted`. Global denies still apply. See [../2_design.md §2](../2_design.md#2-profile-resolution). |
| **Process-group leader** | A spawned child whose PGID equals its PID, set via `command.process_group(0)` on Unix, so `killpg` can reap orphan subprocesses through the same group. See [../2_design.md §7](../2_design.md#7-sandbox--exec-primitives). |
| **Resolved profile** | `ResolvedFsProfile { name, read, modify }` — the post-resolution shape that the evaluator walks. Different from the raw `FsProfile` because deny rules are already injected as negated entries. See [../2_design.md §2](../2_design.md#2-profile-resolution). |
| **Sandbox trait** | The `Sandbox::validate(req)` seam in `orbit-exec` where a future OS-level isolation impl would attach. The default `NoSandbox` always returns `Ok`. See [../2_design.md §7](../2_design.md#7-sandbox--exec-primitives). |
| **Termination escalation** | The SIGTERM → 5-second grace → SIGKILL sequence applied to a child process group on timeout or parent-signal interruption. See [../2_design.md §8](../2_design.md#8-process-supervision). |
| **Tool-layer enforcement** | Orbit's policy enforcement seam for HTTP-backed activities: every fs builtin calls `enforce_fs_policy` before the underlying read or modify, and emits `FsCallEvent` regardless of allow/deny outcome. CLI-backed activities bypass this seam entirely. The exec layer does not enforce policy. See [../2_design.md §5](../2_design.md#5-tool-layer-enforcement). |
