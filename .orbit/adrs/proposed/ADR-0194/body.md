## Context
The v2 `shell` activity (`ActivityV2Spec::Shell`) dispatched `Command::new(program)` with no OS sandbox, no cwd confinement, and no policy consultation — unlike the `backend: cli` agent path. Its only guard was a `program in allowed_programs` check where both fields came from the same workspace-supplied YAML, making the allowlist a tautology (ORB-00363). The real alternatives were to retrofit the sandbox/policy/cwd pipeline onto `run_shell`, or to remove the surface entirely.

## Decision
Remove the `shell` activity surface end to end: drop `ShellSpec`, `ActivityV2Spec::Shell`, `run_shell`, the `Shell*` `DispatchError` variants, and every match arm, re-export, demo asset, and doc reference. A workspace activity/job declaring `type: shell` now fails to deserialize at load (the `#[serde(tag = "type")]` enum has no matching variant) instead of executing. Narrow subprocess needs are served by registered `deterministic` actions and the policy-gated `backend: cli` agent path, which enforces `proc_allowed_programs` outside the workspace-supplied spec.

## Consequences
- A malicious or careless workspace can no longer obtain unsandboxed arbitrary-program execution through a self-asserted allowlist; the failure mode is fail-closed (load error), not silent execution.
- The only built-in dispatch leaf that produced `Ok(success = false)` is gone; every remaining leaf returns `Ok(success = true)` or `Err`. The structural non-success propagation in the job executor (`StepOutcome.success`, parallel / fan-out / loop aggregation) is retained as the general contract for block-level outcomes and any future fallible-but-`Ok` activity.
- No single code anchor: the constraint is the absence of the variant, enforced by the typed `ActivityV2Spec` enum and review.
- Cost: the `Ok(success = false)` audit-message path lost its only coverage — the two shell-specific tests asserting it were removed rather than migrated, because `deterministic` actions cannot produce that outcome.
- Cost: workspaces that legitimately used `type: shell` must migrate to a registered `deterministic` action or an `agent_loop`; there is no compatibility shim, and old YAML fails at load.