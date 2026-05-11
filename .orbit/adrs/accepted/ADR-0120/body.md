## Context
The companion binary is installed outside the main `orbit` executable, so upgrading Orbit does not automatically replace an already-present `~/.orbit/embed/bin/orbit-embed-companion-<platform>`. A stale companion can therefore keep old subprocess behavior after the main binary has moved on. The concrete failure was a stale companion writing `execution failed: Broken pipe (os error 32)` to stderr during best-effort background task indexing, after the durable task update had already succeeded. Direct semantic commands should still surface companion stderr because users explicitly invoked the semantic subsystem and need useful failure detail.

## Decision
`orbit semantic install` probes an existing installed companion with `--version-info` and compares the returned version to the current Orbit package version. Missing, stale, unprobeable, or explicitly forced companions are replaced through a temporary sibling file before being moved into place; successful install output reports `companion_changed`. The CLI exposes `--force` for intentional replacement even when the probe says the companion is current. `SubprocessEmbedder` keeps inherited stderr as the default for direct semantic commands, while the background task-mutation worker uses a quiet spawn mode.

## Consequences
- Re-running `orbit semantic install` after upgrading Orbit naturally refreshes stale companions without requiring users to uninstall first.
- Task mutation output stays trustworthy: background indexing remains best-effort and cannot leak companion stderr into successful `task.add` / `task.update` command output.
- Direct commands such as `orbit semantic search`, `related`, and `reindex` still show actionable companion stderr because they use the inherited-stderr path.
- Cost: install now trusts the companion's `--version-info` protocol. If a broken companion cannot answer the probe, Orbit conservatively replaces it, which can redownload or recopy the binary even when the file might have been usable for embeddings.

---

## Task References

- [T20260510-3] — Design semantic search over task artifacts and graph (v2). The task that produced this folder.
- [T20260510-9] — Phase-1 semantic search foundation: orbit-embed + orbit-embed-companion + indexing pipeline. The task that accepted and implemented ADR-001 through ADR-006.
- [T20260510-20] — Refactor: relocate semantic-search ownership to orbit-embed (vector store + commands). The task that accepted and implemented ADR-007.
- [T20260510-26] — Make semantic companion install/update quiet and version-aware. The task that accepted and implemented ADR-008.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
