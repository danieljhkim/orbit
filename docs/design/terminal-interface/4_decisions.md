---
title: Terminal Interface — Decisions
owner: claude
last_updated: 2026-08-02
last_validated: 2026-08-02
status: Accepted
feature: terminal-interface
doc_role: decisions
type: design
summary: "Ordered ADR pointer index for orbit's terminal output surface."
tags: [terminal-interface]
paths: ["crates/orbit-cli/src/output/**"]
related_features: [terminal-interface]
related_artifacts: [ADR-0306, ADR-0307, ADR-0308, ADR-0314]
---

# Terminal Interface — Decisions

Ordered pointer index for terminal-interface's ADRs. **Allocate the global `ADR-NNNN` via `orbit.adr.add` before adding the pointer** — never hand-author a four-digit number. The store owns the title, status, body, owner, and links; retrieve an ADR's authoritative narrative with `orbit tool run orbit.adr.show --input '{"id":"ADR-NNNN"}'`. See [CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict) for the full rules (when a decision earns an ADR, the mandatory Cost line, rollups).

ADR-0306, ADR-0307, and ADR-0308 were accepted on 2026-08-02, ahead of the implementation work that makes the code conform. That ordering is deliberate — the specs in [./specs/](./specs/) are the reviewed contract, and [2_design.md](./2_design.md) records per-mechanism where the code still diverges from it. A divergence noted there is scheduled work, not an unmade decision. ADR-0314 is the inverse case: the implementation ([ORB-10570]) landed first, and the ADR was filed after, once the store was writable again.

- **ADR-0306 — Terminal Output Is a Rendering of a Structured Payload** — Accepted. Commands produce payloads; a central renderer resolves mode (`auto|table|json|ndjson`) from flags and TTY state. Generalizes the error payload shape established by [ORB-10356]. Implemented by [./specs/output-modes.md](./specs/output-modes.md).
- **ADR-0307 — Borderless Tables With Truncate-to-Width Rows** — Accepted. Drops the box preset; a row is exactly one line, truncated with `…` and backed by a detail command. Reverses the wrapping arrangement introduced by [T20260411-0335]. Implemented by [./specs/table-rendering.md](./specs/table-rendering.md).
- **ADR-0308 — One Semantic Color Vocabulary, Gated at the Sink** — Accepted. A closed role set replaces the two duplicated palettes; `NO_COLOR`, `--color`, and TTY state are resolved once. Consolidates the vocabulary [T20260427-43] extended in two places at once. Implemented by [./specs/color-and-styling.md](./specs/color-and-styling.md).
- **ADR-0314 — The Output Sink Is a Process Global, Not a Renderer Parameter** — **Superseded** by [ORB-10586], which performed the 154-impl signature change ADR-0314 deferred. Its rejected alternative ("thread the sink through `Execute::execute`") became the accepted one once commands returned payloads: `main` passes the sink to the single renderer, and `sink::install`/`sink::active` are deleted. The supersession is a change of premise, not of judgment — ADR-0314 was correct while `Execute::execute` had no renderer argument. The superseding ADR is allocated by the orchestrator; its body is drafted in [ORB-10586]'s execution summary. See [2_design.md](./2_design.md) §9.

## Task References

- [T20260411-0335] — introduced the table arrangement [ADR-0307] reverses.
- [T20260427-43] — extended the duplicated color vocabulary [ADR-0308] consolidates.
- [ORB-10356] — made `OrbitError` `#[non_exhaustive]`, establishing the error payload shape [ADR-0306] generalizes.
- [ORB-10570] — wired the sink into color and width, the implementation [ADR-0314] documents.
- [ORB-10585] — filed [ADR-0314], which [ORB-10570] could not allocate.
- [ORB-10586] — converted every command body to return a payload and gave the renderer sole ownership of stdout ([ADR-0306] steps 2–4), superseding [ADR-0314].

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
