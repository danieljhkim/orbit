---
title: Terminal Interface — Decisions
owner: claude
last_updated: 2026-08-01
last_validated: 2026-08-01
status: Draft
feature: terminal-interface
doc_role: decisions
type: design
summary: "Ordered ADR pointer index for orbit's terminal output surface."
tags: [terminal-interface]
paths: ["crates/orbit-cli/src/output/**"]
related_features: [terminal-interface]
related_artifacts: [ADR-0306, ADR-0307, ADR-0308]
---

# Terminal Interface — Decisions

Ordered pointer index for terminal-interface's ADRs. **Allocate the global `ADR-NNNN` via `orbit.adr.add` before adding the pointer** — never hand-author a four-digit number. The store owns the title, status, body, owner, and links; retrieve an ADR's authoritative narrative with `orbit tool run orbit.adr.show --input '{"id":"ADR-NNNN"}'`. See [CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict) for the full rules (when a decision earns an ADR, the mandatory Cost line, rollups).

All three entries below are `Proposed`. They were allocated when the house style was written, ahead of the implementation work that will make the code conform — the divergences are recorded per-mechanism in [2_design.md](./2_design.md). Flip each to `Accepted` via `orbit.adr.update` when its implementing task lands, then refresh the status here.

- **ADR-0306 — Terminal Output Is a Rendering of a Structured Payload** — Proposed. Commands produce payloads; a central renderer resolves mode (`auto|table|json|ndjson`) from flags and TTY state. Generalizes the error payload shape established by [ORB-10356]. Implemented by [./specs/output-modes.md](./specs/output-modes.md).
- **ADR-0307 — Borderless Tables With Truncate-to-Width Rows** — Proposed. Drops the box preset; a row is exactly one line, truncated with `…` and backed by a detail command. Reverses the wrapping arrangement introduced by [T20260411-0335]. Implemented by [./specs/table-rendering.md](./specs/table-rendering.md).
- **ADR-0308 — One Semantic Color Vocabulary, Gated at the Sink** — Proposed. A closed role set replaces the two duplicated palettes; `NO_COLOR`, `--color`, and TTY state are resolved once. Consolidates the vocabulary [T20260427-43] extended in two places at once. Implemented by [./specs/color-and-styling.md](./specs/color-and-styling.md).

## Task References

- [T20260411-0335] — introduced the table arrangement [ADR-0307] reverses.
- [T20260427-43] — extended the duplicated color vocabulary [ADR-0308] consolidates.
- [ORB-10356] — made `OrbitError` `#[non_exhaustive]`, establishing the error payload shape [ADR-0306] generalizes.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
