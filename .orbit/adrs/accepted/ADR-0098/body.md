## Context
Policy enforcement could plausibly live at the syscall layer, the fs trait layer, the tool layer, or the activity layer. Each placement has different trust and coverage tradeoffs.

## Decision
Enforcement lives in `orbit-tools::builtin::fs::enforce_fs_policy`. Every fs builtin calls it before the underlying read or modify, and emits `FsCallEvent` through `FsAuditLogger`. The `Sandbox` trait in `orbit-exec` does not consult the policy engine; exec is supervised but not policy-gated. This applies only to `backend: http` activities — `backend: cli` runs spawn an external CLI agent and emit a `tool_allowlist.harness_delegated` event in lieu of enforcement.

## Consequences
- HTTP-backed fs decisions have one auditable helper, but tool authors must route work through it.
- Cost: CLI-backed activities still bypass this helper, and HTTP tools that skip it are also unguarded. Current macOS executors can narrow CLI filesystem writes with `sandbox-exec`, but closing the general gap likely requires a `PolicyAwareFs` trait, broader OS sandboxes, or both.
