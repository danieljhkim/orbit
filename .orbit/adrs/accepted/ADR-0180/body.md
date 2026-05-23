## Context
Doc search was lexical-only after ORB-00202 unified the query surface, while the orbit-search store already had a source_kind discriminator that could hold docs. The alternatives were to keep semantic ranking deferred, add a separate docs search verb, or reuse the existing vector store behind the unified `orbit search --kind doc --hybrid` path.

## Decision
Use `orbit docs index` as the explicit admin verb that embeds configured docs roots into `source_kind = "doc"` rows, and keep retrieval opt-in through `orbit search <query> --kind doc --hybrid`. Lexical doc search remains the default, ADRs stay lifecycle-owned and lexical-only, and `[docs.search].semantic_weight` tunes the blend without adding another CLI flag.

## Consequences
- The old no-op docs indexing verb is retired rather than kept as a shim, so the docs lifecycle verb now matches `orbit semantic index`.
- Doc embeddings reuse orbit-search storage and companion model selection without adding an orbit-search to orbit-core dependency.
- Hybrid doc search can improve concept queries while preserving lexical fallback when the companion or doc rows are unavailable.
- Cost: the docs index becomes a second freshness loop next to task semantic indexing; operators must run `orbit docs index` after substantial doc moves or edits until background indexing exists.