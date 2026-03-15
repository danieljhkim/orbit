# Execution Summary - Remove ANTHROPIC_API_KEY from Claude Agent Required Env Vars
Agent Name: Claude
Agent Model: claude-sonnet-4-6

## Status
success

## Orbit Task
Task ID: T20260315-045925

## 1. Summary of Changes
Removed `ANTHROPIC_API_KEY` from Claude provider's hermetic env requirements. Three files changed:

- `orbit-agent/src/providers/mod.rs` — `AgentProvider::Claude` required_env_vars reduced from `["HOME", "PATH", "ANTHROPIC_API_KEY"]` to `["HOME", "PATH"]`
- `orbit-agent/tests/protocol_behavior.rs` — updated `claude_runtime_declares_required_env_vars` assertion; renamed `claude_runtime_requires_anthropic_api_key` to `claude_runtime_does_not_require_anthropic_api_key` and inverted the assertion
- `orbit-core/tests/job_runtime_behavior.rs` — removed `ANTHROPIC_API_KEY` from hermetic pass list in claude mock test; removed `set_var`/`remove_var` env setup and the now-unnecessary `env_lock` guard around the claude job run test

## 2. Strategic Decisions
- Keep PATH in required vars | Rationale: Claude CLI binary lookup still depends on PATH | Trade-offs: none
- Invert the old test rather than delete it | Rationale: makes the new contract explicit and prevents future regressions that add the key back | Trade-offs: slightly more code, but self-documenting

## 3. Assumptions Made
- Claude Code resolves credentials from `~/.claude/` via HOME | Impact if incorrect: claude jobs would fail with an auth error at runtime, not at env-check time
- No other code path reads `required_env_vars` and gates on `ANTHROPIC_API_KEY` presence | Impact if incorrect: gating logic would silently stop blocking; verified by grepping — only the provider definition and its tests referenced it

## 4. Design Weaknesses / Risks
- If a user sets up API-key auth instead of subscription auth, claude jobs will not proactively fail the env check; they will fail later at runtime | Severity: Low | Mitigation: runtime error from claude CLI will still surface the issue clearly

## 5. Deviations from Original Plan
- None

## 6. Technical Debt Introduced
- None

## 7. Recommended Follow-Ups
- Consider documenting in `orbit.toml` config reference that Claude uses HOME-based credential resolution (not ANTHROPIC_API_KEY)

## 8. Overall Assessment
Minimal, clean change. Three targeted edits, all tests green (138 passing). Subscription-based Claude auth now works without any env var workarounds.