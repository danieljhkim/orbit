## Context

The decay checker shipped as `scripts/check_design_doc_decay.py` (117 lines) wrapped by `make check-design-docs`. Three properties of that placement bothered enough to file ORB-00019: (a) agents driving Orbit through MCP could not invoke the check without shelling out to the make target, (b) the python parser duplicated logic that belonged with the rest of the design-doc tooling (which was about to grow scaffolding and inspection), and (c) the script was not exercised by Orbit's own integration tests, so a parser bug could ship undetected. Three options were on the table: keep python and just expose it through MCP via a shim; rewrite in Rust as a CLI only; rewrite in Rust with both a CLI and an `orbit.design.*` MCP tool surface.

## Decision

Rewrite in Rust as `orbit-core::command::design` with both an `orbit design check` CLI and the `orbit.design.{init,list,show,check}` MCP surface. Reduce `scripts/check_design_doc_decay.py` to a thin wrapper that shells out to the new CLI so existing references to the script path keep working. Wire `make check-design-docs` to invoke `cargo run -- design check`.

## Consequences

- Agents driving Orbit through MCP can scaffold and check design folders without shelling out, which makes "skip the docs" the harder choice in agent-driven workflows.
- The decay-check logic gets covered by the workspace test suite; output equivalence with the python script was verified end-to-end before the python code was deleted.
- The init/list/show surface enables future tooling (lint, semantic search, glossary index, [3_vision.md §1.2](./3_vision.md)–[§1.7](./3_vision.md)) to extend a Rust API rather than fork a python script.
- Cost: more code to maintain than 117 lines of python (~700 lines of Rust across `orbit-core::command::design`, the CLI shim, the four MCP tool registry entries, and the dispatch). The boundary between `orbit-core` and `orbit-cli` had to be plumbed for the new command. The python wrapper survives as a backwards-compatibility seam that has to be kept in sync with the CLI invocation contract; if `orbit design check` ever changes its flag set, the wrapper has to follow.