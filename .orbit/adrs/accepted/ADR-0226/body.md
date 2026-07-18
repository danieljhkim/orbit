## Context
Cross-machine work needs a coordination authority, a knowledge author, and an execution destination; treating them as the same authority would scatter task state or make ownership implicit.
## Decision
Use exactly one coordination hub for every workspace, declare one workspace owner, and select execution placement per run with the owner as the default.
## Consequences
- Coordination writes remain hub-routed while knowledge authorship remains owner-bound.
- Cost: hub downtime stalls coordination for every workspace, and disconnected machines cannot write coordination records.