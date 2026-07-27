## Context

The companion binary is installed outside the main `orbit` executable, so upgrading Orbit does not automatically replace an already-present `~/.orbit/embed/bin/orbit-embed-companion-<platform>`. A stale companion can therefore keep old subprocess behavior after the main binary has moved on. The concrete failure was a stale companion writing `execution failed: Broken pipe (os error 32)` to stderr during best-effort background task indexing, after the durable task update had already succeeded. Direct semantic commands should still surface companion stderr because users explicitly invoked the semantic subsystem and need useful failure detail.

## Decision

`orbit semantic install` probes an existing installed companion with `--version-info` and compares the returned version to the current Orbit package version. Missing, stale, unprobeable, or explicitly forced companions are replaced through a temporary sibling file before being moved into place; successful install output reports `companion_changed`. The CLI exposes `--force` for intentional replacement even when the probe says the companion is current. `SubprocessEmbedder` keeps inherited stderr as the default for direct semantic commands, while the background task-mutation worker uses a quiet spawn mode.

## Consequences


- Re-running `orbit semantic install` after upgrading Orbit naturally refreshes stale companions without requiring users to uninstall first.
- Task mutation output stays trustworthy: background indexing remains best-effort and cannot leak companion stderr into successful `task.add` / `task.update` command output.
- Direct commands such as `orbit search <query> --hybrid`, `orbit search similar <task-id>`, and `orbit semantic index` still show actionable companion stderr because they use the inherited-stderr path.
- Cost: install now trusts the companion's `--version-info` protocol. If a broken companion cannot answer the probe, Orbit conservatively replaces it, which can redownload or recopy the binary even when the file might have been usable for embeddings.

## Provenance

Migrated verbatim from the local heading `orbit-search/ADR-008` in `docs/design/orbit-search/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-05 · [T20260510-26]