## Context

Folder naming is a low-stakes choice in isolation but high-stakes once cross-links accumulate. Options on the table: `PascalCase`, `snake_case`, `kebab-case`; singular vs plural; whether to allow nested folders.

## Decision

Folder names are lowercase, hyphenated (`kebab-case`), singular: `knowledge-graph` not `KnowledgeGraph`, `policy-sandbox` not `policy_sandbox`, `task-artifacts` (which deliberately reads as "the family of task artifacts" but stays singular as a folder concept). No nesting; every feature is a sibling under `docs/design/`. Folder names matching `_*` (e.g. `_archive/`) are reserved for retired-feature storage and are skipped by tooling.

## Consequences

- Cross-link paths are uniform and predictable.
- The retirement path (`mv docs/design/foo docs/design/_archive/foo`) is a one-line operation that tooling respects automatically.
- Cost: renaming a feature folder breaks every link to its docs. The fix is mechanical (grep + sed) but requires touching every doc that referenced the old name. There is no rename redirect mechanism. Practically, folder names are very rarely renamed once established; the cost has been paid twice in two years.