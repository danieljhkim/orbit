## Context
The macOS CLI wrapper previously spawned `sandbox-exec` by bare name and checked availability by walking `PATH`. A writable or config-influenced `PATH` could point Orbit at an attacker-controlled wrapper while Orbit still believed kernel sandbox enforcement was active.

## Decision
Resolve the wrapper only from trusted absolute locations, currently `/usr/bin/sandbox-exec`, and use the same trusted resolver for availability checks, audit argv, and process spawn. Missing trusted binaries fail closed unless the executor explicitly allows fallback, and the error names the trusted location that was probed.

## Consequences
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

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
