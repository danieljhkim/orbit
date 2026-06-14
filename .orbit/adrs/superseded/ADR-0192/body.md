## Context
ORB-00338 cut the active graph query tools over from orbit-knowledge to orbit-graph, but audit data and post-cutover testing found unacceptable steady-state regressions: p50 search slowed from 694ms to 9,349ms, every new query tool inherited a roughly 9s cold-call floor from `SyncPolicy::Windowed { window: 500ms }`, four heavily used tools (`pack`, `overview`, `implementors`, `deps`) disappeared, plugin MCP exposure missed the registered surface, `show` returned bytes to LLM consumers, `trace` returned empty for real enum-dispatch commands, and `impact` mixed forward/reverse traversal. The real alternatives were to keep patching the cutover in place or roll back active routing while retaining the orbit-graph crate and harness for a better attempt.

## Decision
Roll back the ORB-00338 active tool routing and restore orbit-knowledge-backed `orbit.graph.search`, `show`, `refs`, `callers`, `pack`, `overview`, `implementors`, and `deps` as the primary graph surface. Keep the orbit-graph crate and equivalence harness in tree, supersede ADR-0191, and treat ORB-00339 through ORB-00343 as obsolete standalone work whose insights become gates for the next cutover attempt.

## Consequences
- Query tools return to the legacy orbit-knowledge performance envelope; future query entry points must default to `SyncPolicy::Manual` unless a measured long-lived process explicitly opts into another policy.
- Pre-cutover audit-log analysis is mandatory before any backend swap so real tool usage is enumerated and replacements exist for heavily used surfaces.
- Plugin MCP exposure must have an equivalence test proving the advertised graph tool set matches the registered active set before a backend swap lands.
- Response wire format must decide text versus bytes up front: UTF-8 `String` at the response boundary, with byte fallback only for genuine non-UTF-8 content.
- Trace and impact gates must cover full enum-dispatch subcommand variants and direction-filter semantics, not only toy fixtures.
- The equivalence harness must capture cold-call latency on a representative corpus in addition to warm-cache correctness.
- Lost for now: cutover-only `callees`, `impact`, `trace`, the changed `sync` shape, and the extended graph-equiv corpus; reintroduce them only through follow-up tasks with the gates above.
- No single code anchor; the rollback decision spans tool registration, MCP exposure, docs, and validation and is enforced through review plus focused surface tests.
- Cost: orbit-graph remains in tree but is paused as the active graph backend, so agents temporarily lose the new cutover-only APIs until the root causes are fixed and a new cutover passes the gates.