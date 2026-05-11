## Context
Some decisions don't belong to a single feature. CLAUDE.md today carries many of them: error-handling conventions, async-locking rules, design-doc reading discipline. Three options for homing cross-cutting ADRs: (a) duplicate them across every relevant feature folder (rot risk); (b) pick one folder arbitrarily — "first in `related_features` wins" (arbitrary, fragile); (c) introduce a dedicated `cross-cutting` pseudo-feature with its own index.

## Decision
Option (c). `docs/design/cross-cutting/` exists as a pseudo-feature folder. Its generated `4_decisions.md` lists every ADR with `cross-cutting` in `related_features`. Per-feature indexes also include cross-cutting ADRs that touch their feature (via the existing `--feature` filter, which matches any element of `related_features`). The folder holds only `1_overview.md` (short description of what cross-cutting means) and the generated `4_decisions.md` — no `2_design.md` or `3_vision.md`, since the folder describes a class of decisions, not a feature.

## Consequences

- Cross-cutting decisions have a canonical home with no duplication.
- Feature folder indexes still show cross-cutting decisions that touch them, so readers don't need to remember to also check `cross-cutting/`.
- CLAUDE.md rules that earn ADR status (after the §1.2 follow-up sweep) migrate into this folder over time. CLAUDE.md remains the high-density rules summary; cross-cutting ADRs are the durable record behind each rule.
- Cost: `docs/design/cross-cutting/` doesn't follow the standard four-numbered-doc layout. CONVENTIONS.md §3 gains a documented exception for pseudo-features, or a small dedicated section. This is the kind of one-off carve-out that conventions docs accumulate; the alternative (forcing a vision/design doc onto a folder that has no design) is worse.

---
