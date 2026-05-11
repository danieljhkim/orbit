## Context
The biography surface (§6 of [2_design.md](./2_design.md)) renders structured edge data into a prose narrative the agent reads in one shot. Two rendering shapes were on the table. Shape A: a templated renderer that pulls strings from canonical task fields and arranges them per a fixed grammar derived from the edge taxonomy. Shape B: an LLM summarizer that paraphrases the structured data into more readable prose. Shape B is more polished; Shape A is more auditable.

## Decision
Shape A in Phase 1. The renderer is deterministic and templated — no LLM in the path. Each rendered line traces back to a specific edge and a specific source row (commit SHA, ADR section, task field). Shape B is deferred to Phase 3 behind a feature flag, and only after the deterministic renderer has shipped and the substrate has proved load-bearing.

## Consequences
- Every claim in a rendered biography is auditable. An agent that doubts a sentence can resolve the underlying edge and read the source row directly.
- Wrong derivations show up in the biography as wrong sentences, and the fix is at the deriver — not at the renderer, not at the edge row. This preserves the feedback loop named in [ADR-003](#adr-003--derivation-first-edges-declared-edges-are-the-exception).
- Biographies will read more stiffly than an LLM summary would. For a feature whose target consumer is *agents* (not humans skimming a sidebar), this is acceptable; agents do not need polish to absorb context.
- Cost: a templated renderer is more rigid. Some cross-task narrative arcs that would read naturally as paraphrase ("the team walked back from approach X over three tasks") render mechanically as three separate paragraphs. The polish gap is real and is the price of audit fidelity. If the gap proves a usability blocker after Phase 1 lands, the Phase 3 LLM-summarized layer becomes the resolution, gated on the deterministic renderer continuing to be the source of truth underneath.

---
