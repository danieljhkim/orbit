## Context

Orbit's code graph grew into a large, specialized subsystem whose maintenance and product surface no longer justified its utility. ORB-10357 removed the `orbit graph` command and all agent-facing graph surfaces, consolidated the remaining implementation into a dependent-free `orbit-graph` crate, and explicitly parked that crate for deletion. The active runtime still contains several vestiges: symbol `context_files` emit a graph-availability warning despite validating only their file anchor, `orbit doctor` and detailed dashboard health probe graph databases that Orbit can no longer build or use, stale graph databases may remain under `.orbit/graph` and `.orbit/knowledge/graph`, `[graph] editing` is still accepted as inert configuration, and graph-specific benchmark suites remain in tree.

Older accepted decisions—ADR-0190, ADR-0195, ADR-0197, ADR-0198, ADR-0241, and ADR-0285—describe graph storage, watcher, benchmark, or product surfaces that the retirement removes. ADR-0285's body records the product-surface reversal, but its lifecycle status and the other records were not fully reconciled.

## Decision

Retire the code graph as an Orbit capability and delete the remaining subsystem rather than preserve dormant compatibility surface.

1. Orbit has no graph CLI, MCP tools, tool-registry entries, execution guidance, health/readiness checks, or runtime behavior.
2. A `symbol:` task context selector is a canonical selector whose file anchor is checked for workspace containment and existence. Orbit does not resolve or validate symbol existence, and it emits no graph-availability warning.
3. `orbit doctor` does not diagnose graph indexes. It may expose the explicit, opt-in `--remove-graph` maintenance action to delete retired graph state from the exact workspace locations `.orbit/graph` and `.orbit/knowledge/graph`; ordinary doctor runs remain read-only and graph-unaware.
4. The dependent-free `orbit-graph` crate, the inert `[graph] editing` configuration key, and the graph-specific `benchmarks/graph/` and `benchmarks/graph-latency/` suites are deletion targets, not compatibility commitments. Non-graph benchmark suites remain. Physical deletion is a separate bounded task from the selector/doctor cleanup so each change remains reviewable.
5. Historical graph design documents may remain archived for provenance, but active documentation must not describe graph as a supported Orbit capability.

This decision supersedes ADR-0190, ADR-0195, ADR-0197, ADR-0198, ADR-0241, and ADR-0285.

## Consequences

- Orbit's supported navigation path is filesystem reading/search plus `orbit search`; call-graph queries are no longer provided.
- Task selector behavior becomes honest and deterministic: symbol fragments remain descriptive metadata while validation is file-anchor based.
- Health output no longer reports an unavailable subsystem, and operators receive one explicit cleanup path for retired graph data.
- Removing the isolated crate, graph-only benchmark suites, and inert configuration reduces code, dependencies, configuration, and maintenance burden without adding a replacement abstraction or feature flag.
- Cost: Orbit loses built-in call-graph navigation, the ability to reuse old graph databases, and the in-tree reproducible graph benchmark artifacts. This is accepted because the graph has no remaining callers and its observed utility does not justify its footprint; git history and ADRs retain provenance.
- Cost: `--remove-graph` permanently deletes derived graph state. The action is opt-in, targets only fixed Orbit-owned directories, and is idempotent; no deletion occurs during an ordinary doctor run.