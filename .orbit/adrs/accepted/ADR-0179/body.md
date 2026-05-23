## Context
ADR-0175 corrected the search flag names after phase 1, but the resulting CLI still mixed a positional query with mode flags and allowed flat status tokens whose meaning changed by corpus kind. The real alternatives were to keep extending that single-command flag matrix, or split the user-facing CLI modes before more corpora grow vector support.

## Decision
Use three explicit CLI forms: `orbit search <query>` for free-text search, `orbit search similar <id>` for cosine-neighbor lookup, and `orbit search path <path>` for applicability lookup. Require `--status` values to use `kind:value` tokens such as `task:open`, `doc:active`, and `adr:proposed`. Remove the CLI field-selection and embedding-model flags, and remove the parallel MCP `field` and `embedding_model` parameters while keeping MCP `model` only as provenance.

## Consequences
- The CLI no longer has a top-level `<query | --semantic | --path>` trichotomy; each primary search operation has its own visible form.
- Status filters are unambiguous across task, doc, learning, and ADR corpora.
- MCP remains a parameterized tool surface, but it mirrors the reduced public parameter set and the same per-kind status parser.
- Cost: `similar` and `path` become reserved words immediately after `orbit search`; searching those literal words requires passing a quoted/free-text query with additional context.
- Cost: callers using the young mode flags, flat `--status`, the retired CLI field/model flags, MCP `field`, or MCP `embedding_model` surfaces must migrate with no compatibility shim.