---
title: Terminal Interface — Overview
owner: claude
last_updated: 2026-08-01
last_validated: 2026-08-01
status: Accepted
feature: terminal-interface
doc_role: overview
type: design
summary: "House style for orbit-cli terminal output — machine-readable first, borderless single-line tables, semantic color resolved at the sink."
tags: [terminal-interface]
paths: ["crates/orbit-cli/src/output/**", "crates/orbit-cli/src/command/**"]
related_features: [terminal-interface, user-interface]
related_artifacts: [ADR-0306, ADR-0307, ADR-0308]
---

# Terminal Interface — Overview

Terminal Interface owns what `orbit` writes to stdout and stderr: table layout, color, truncation, output modes, progress, and errors. It is the CLI counterpart to [user-interface](../user-interface/1_overview.md), which owns the web dashboard — the two surfaces share operator vocabulary and status semantics but no code, no tokens, and no rendering assumptions. Its governing claim is that terminal output has two audiences with opposed needs, and that serving both is a matter of resolving which one is present rather than compromising between them.

## 1. Motivation

A CLI's output is read by a human scanning for a status and by a pipeline extracting a field, and those readers want different bytes. The human wants alignment, color, and a screenful; the pipeline wants one record per line and no escape sequences. Orbit currently sends both readers the same bytes, chosen for the human, and the pipeline absorbs the mismatch.

The concrete failures are small and additive. `orbit tool list` draws box rules and wraps long cells across three or four lines, so a row is not a line and `grep` returns fragments ([T20260411-0335] introduced the full-width dynamic arrangement that causes the wrapping). `orbit audit list` aligns its columns with literal padding widths in a format string, so a value wider than the literal breaks the column. `NO_COLOR` is honored on the paths that use the `colored` crate and ignored on the paths that use `comfy-table`. Structured output is a per-command opt-in present on 86 of 150 argument structs. Nothing anywhere asks whether stdout is a terminal, except one log-tailing command that asks locally.

None of these is severe alone. Together they mean output correctness is a property each command has or lacks individually, which is the thing this feature exists to stop.

## 2. Core Concepts

- **Payload** — the structured record a command produces. The contract; both renderings derive from it [ADR-0306].
- **Renderer** — the layer that projects a payload into bytes. The only code that knows about terminals, width, or ANSI.
- **Output mode** — `auto` | `table` | `json` | `ndjson`. Resolved centrally from flags and TTY state, never per command.
- **Role** — a semantic color token (`ok`, `warn`, `error`, `active`, `muted`, `neutral`). Commands tag values with roles; only the renderer maps roles to color [ADR-0308].
- **Sink** — the resolved stdout target plus its capabilities (is it a TTY, how wide, may it carry ANSI). Every environment question is answered here once.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Table construction, preset, arrangement | [crates/orbit-cli/src/output/table.rs](../../../crates/orbit-cli/src/output/table.rs) | [T20260411-0335] |
| Status and priority color vocabulary | [crates/orbit-cli/src/output/color.rs](../../../crates/orbit-cli/src/output/color.rs) | [T20260427-43] |
| Structured output and error payloads | [crates/orbit-cli/src/output/json.rs](../../../crates/orbit-cli/src/output/json.rs) | [ORB-10356] |
| Hand-padded audit line output | [crates/orbit-cli/src/command/audit/support.rs](../../../crates/orbit-cli/src/command/audit/support.rs) | [ORB-10228] |
| Per-command rendering (current, non-conforming) | [crates/orbit-cli/src/command/](../../../crates/orbit-cli/src/command/) | [ORB-00279] |
| Target table contract | [./specs/table-rendering.md](./specs/table-rendering.md) | [ADR-0307] |
| Target color contract | [./specs/color-and-styling.md](./specs/color-and-styling.md) | [ADR-0308] |
| Target mode-resolution contract | [./specs/output-modes.md](./specs/output-modes.md) | [ADR-0306] |

## Task References

- [T20260411-0335] — introduced the dynamic full-width table arrangement for narrow terminals.
- [T20260427-43] — added a `friction` arm to both halves of the duplicated color vocabulary.
- [ORB-00279] — flattened the `orbit-cli` command tree into the current per-command layout.
- [ORB-10228] — added trusted MCP session context to the audit event JSON payload, but not to the printed line.
- [ORB-10356] — made `OrbitError` `#[non_exhaustive]`, adding the `internal_error` catch-all to the payload's `code` discriminator.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
