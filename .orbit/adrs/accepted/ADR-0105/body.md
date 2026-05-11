## Context
ADR-012 unblocked Codex state writes, but Claude writes startup state under `$HOME/.claude` or `$CLAUDE_CONFIG_DIR`, and Gemini writes under `$HOME/.gemini`. SBPL compilation receives `ResolvedFsProfile` plus host env, not the active provider, so provider-conditional allow clauses would require new plumbing.

## Decision
Emit state-dir allows for all supported CLI providers on every macOS sandbox profile: `$CODEX_HOME` / `$HOME/.codex`, `$CLAUDE_CONFIG_DIR` / `$HOME/.claude`, and `$HOME/.gemini`. Keep `append_provider_side_write_roots` Codex-only because Claude and Gemini have no `--add-dir` equivalent; document that a future provider with such a surface should generalize the branch.

## Consequences
- Claude and Gemini reach past CLI startup under `macos-sandbox-exec` with the same state-dir defense story as Codex.
- Emitting all three narrow state-dir allowances avoids provider plumbing; Codex side roots remain a separate branch until another provider ships an equivalent surface.
- Cost: every macOS sandbox profile carries three state-dir allow clauses regardless of which provider runs. If a future provider's state dir overlaps with another sensitive root, this design needs revisiting.
