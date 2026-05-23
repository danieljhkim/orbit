---
summary: "User Interface — Overview"
type: design
title: "User Interface — Overview"
owner: gemini
last_updated: 2026-04-30
status: Draft
feature: user-interface
doc_role: overview
tags: ["user-interface"]
---

# User Interface — Overview

Orbit UI covers the local dashboard served by `orbit-cli` and the project-facing web surface. It gives operators a dense, legible way to monitor agents, workflows, telemetry, and audit signals.

## 1. Motivation

Agent runs produce more state changes, logs, and diagnostics than a human can read linearly. The UI therefore optimizes for scan density, clear status recognition, and quick drill-downs. Canon Refined keeps the pro-tool feel while using readable sans-serif text, subtle rounding, and restrained semantic color [T20260427-29].

## 2. Core Concepts

- **Canon Refined and typography:** Layered dark surfaces, fine borders, compact spacing, and muted status colors; `Inter` carries labels and prose while `JetBrains Mono` carries IDs, metrics, timestamps, code, and logs.
- **Surfaces:** The local dashboard lives in `crates/orbit-cli/assets/dashboard/`; static docs and project pages should reuse the same visual grammar without importing runtime-only dashboard assumptions.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Local dashboard | `crates/orbit-cli/assets/dashboard/` | Runtime tabs, tables, tiles, logs, and diagnostics. |
| Theme rules | `./specs/theme.md` | Canon Refined tokens and visual invariants. |
| Current mechanisms | `./2_design.md` | Layout, telemetry, palette, typography, and known limitations. |

## Task References

- [T20260427-29] introduced the Canon Refined UI direction.
- [T20260430-24] tightened the UI design docs against shared conventions.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
