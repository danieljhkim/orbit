## Context
ADR-013 modeled Claude's state surface as the `$HOME/.claude/` directory (or `$CLAUDE_CONFIG_DIR` when set) and emitted a single `(allow file-write* (subpath ...))` clause per provider state dir. In practice, Claude Code persists its main settings to `$HOME/.claude.json` — a sibling *file* at the home root, with `.lock` and atomic-write `.tmp.<pid>.<ms_ts>` companions. SBPL `subpath` only matches the named directory and everything strictly below, so `.claude.json` (a sibling, not a child) was denied at the kernel. Symptom: every Claude invocation under `macos-sandbox-exec` lost the ability to update its state, and tool calls that wait on the state-file lock hung silently. Codex/Gemini were unaffected because all of their state lives under their state directories.

The override case is clean: when `CLAUDE_CONFIG_DIR` is set, Claude writes `<override>/.claude.json` and its siblings inside the override directory, already covered by the existing `(subpath "$CLAUDE_CONFIG_DIR")` clause.

## Decision
When the SBPL profile is compiled with `CLAUDE_CONFIG_DIR` unset and `HOME` resolved, additionally emit:

- `(allow file-write* (literal "$HOME/.claude.json"))`
- `(allow file-write* (literal "$HOME/.claude.json.lock"))`
- `(allow file-write* (regex "^$HOME/\.claude\.json\.tmp\.[0-9]+\.[0-9]+$"))`

Use `literal` for the canonical and lock files (predictable names) and `regex` for the tmp pattern. The home prefix in the regex is escaped with the existing `push_regex_escaped` helper so symlink-free home paths containing regex meta characters do not widen the allow.

## Consequences
- Claude under `macos-sandbox-exec` can persist settings and acquire its lockfile; tool calls that depend on a freshly-updated state file no longer hang.
- The `CLAUDE_CONFIG_DIR` branch is unchanged — the existing subpath clause already covers the JSON file inside the override.
- Cost: three additional clauses on every macOS sandbox profile when `HOME` resolves and `CLAUDE_CONFIG_DIR` is unset. Symmetric to the ADR-013 trade-off; provider plumbing is avoided.
- This ADR amends ADR-013 rather than replacing it: the per-provider state-dir clauses still emit unconditionally; the new clauses are scoped to the HOME-fallback branch only.
