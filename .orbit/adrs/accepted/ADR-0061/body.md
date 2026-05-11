## Context
Reference resolution is strongest via a language server, but LSPs are stateful long-running processes tuned for interactive UX. Agent tools want bulk, structured, token-budgeted output and low lifecycle overhead.

## Decision
Use tree-sitter grammars with per-language extractors producing structural symbols only. Defer cross-file reference resolution indefinitely (see [3_vision.md §1.1] for the open question of re-introducing LSP as a pluggable backend). Each new language extends this decision via the table below rather than a new ADR; only language-specific tradeoffs that would surprise a reader (a new `LeafKind` variant, an excluded extension, a non-obvious mapping) earn a row in the Notes column.

| Language | Extensions | Grammar | Task(s) | Notes |
|----------|------------|---------|---------|-------|
| Rust | `.rs` | `tree-sitter-rust` | [T20260406-0455-3], [T20260409-0550] | — |
| Go, Java, JavaScript | `.go`, `.java`, `.js` | upstream | [T20260416-0352] | — |
| Python | `.py` | upstream | (existing) | — |
| TypeScript / TSX | `.ts`, `.mts`, `.cts`, `.tsx`, `.d.ts` | `tree-sitter-typescript` | [T20260505-11] | Adds `enum` and `type_alias` LeafKinds. `.d.ts` classifies through its `.ts` extension. |
| C# | `.cs` | `tree-sitter-c-sharp` | [T20260505-13] | `.csx`, `.cshtml`, and Razor-style files explicitly excluded until separate extractors land. |
| Kotlin | `.kt`, `.kts` | `tree-sitter-kotlin-ng` | [T20260505-14] | Adds `package`, `object`, `companion_object` kinds. Extension functions emit as standalone `function` leaves named `Receiver.function`. |
| Ruby | `.rb` | upstream | [T20260505-15] | — |
| C | `.c`, `.h` | `tree-sitter-c` | [T20260505-16] | Headers share the C extractor; prototypes emit as `function_declaration`. C++-shaped headers stay C-classified until a separate extractor lands. |

## Consequences
- Fast, deterministic extraction with no per-query process lifecycle.
- `find_references`, `callers`, `implementors` ([T20260412-0645-3]) are signature-matched, not type-resolved — a superset of the truth.
- Adding a language is an instance of this ADR, not a new decision: append a row above and cite the task on the Status line.
- Cost: extractor maintenance scales with languages supported, and the graph `LeafKind` surface grows as languages add their own kinds (`enum`, `type_alias`, `package`, `object`, `companion_object`, `function_declaration`); downstream exhaustive matches must absorb each addition while overloads and partial declarations still share syntax-level names until a future signature-aware identity scheme lands.

---
