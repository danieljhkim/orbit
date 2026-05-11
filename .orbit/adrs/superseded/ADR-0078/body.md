## Context
Task-ID attribution was hardcoded to the Orbit format `\[T\d{8}-\d{4}(?:-\d+)?\]` in two places (`pipeline/history.rs` and `service/history.rs`). Codebases using Jira (`PROJ-123`), Linear (`ENG-123`), or GitHub-issue (`#123`) conventions saw empty graph-backed results and a silently empty fallback — the feature was unusable outside Orbit's own repo. The same `orbit task history` CLI surface also lived under the wrong subcommand: it never touches task lifecycle, only the graph.

## Decision
Move the CLI to `orbit graph history <selector>` (drop the redundant `rebuild` subsubcommand; `orbit graph build` does the same job). Introduce a single `TaskIdPattern` accessor in `orbit-knowledge` consumed by both the build-time attribution pass and the history fallback. Expose configuration through `--task-id-pattern <regex>` on `orbit graph build` / `orbit graph history` and `knowledge.task_id_pattern` in workspace `config.toml`, with strict precedence CLI flag > config > Orbit default. Adopt a capture-group convention (group 1 if present, else whole match) so the default Orbit pattern strips brackets in-regex instead of bespoke string slicing — the stored task IDs stay byte-identical to pre-T20260426-0507 graphs. Persist the pattern in `manifest.json`. When the configured pattern differs from the manifest pattern, `orbit graph history` emits a stderr warning, and the attribution pass forces a full-history backfill (cursor reset, prior task_ids hydration skipped) so the new pattern repopulates every node. Also add `orbit.graph.history` as an MCP/agent tool returning the same JSON shape as the CLI's `--json`.

## Consequences
- Non-Orbit codebases can now use the feature without forking; orbit's own repo gets identical output to before.
- A pattern change is safe: a subsequent `orbit graph build` is guaranteed to backfill correctly rather than silently leave stale `task_ids` from the prior pattern.
- The agent tool surface stays in sync with the CLI: a future schema change must update both.
- Cost: a pattern change incurs a full-history walk on next build, and regex validation stays duplicated between orbit-core config and the orbit-knowledge consumer to keep `orbit-core` free of the `orbit-knowledge` dependency; both are deliberate churn to avoid silent stale attribution.

---
