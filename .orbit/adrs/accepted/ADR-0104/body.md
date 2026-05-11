## Context
Codex-backed `agent_implement` reached startup under `sandbox-exec` but failed with `Operation not permitted`: the profile allowed worktree, temp/cache, and `$HOME/.orbit` writes but not Codex state. After that, workflow state still failed because policy denied workspace `.orbit/**` after Orbit passed the same root via Codex `--add-dir`, and `**/*.env` over-denied when compiled as a containing-directory `subpath`.

## Decision
Keep `sandbox-exec` authoritative and add narrow Codex allowances: `$CODEX_HOME` or `$HOME/.codex`, plus Codex side-write roots from runtime provider config appended after policy-derived denials. Compile non-subpath deny globs such as `**/*.env` as SBPL `regex` clauses. Do not grant broad `$HOME` writes or disable the outer sandbox.

## Consequences
- Codex-backed `backend: cli` runs can initialize under the macOS sandbox while project writes stay constrained by the resolved `fsProfile`.
- `CODEX_HOME` relocates state, and inherited Orbit subprocesses can persist workflow state through the same side roots Codex receives.
- Cost: the Codex state directory and provider side roots are trusted writable state outside ordinary project-content policy, similar to the existing `$HOME/.orbit` allowance for inherited Orbit subprocesses.
