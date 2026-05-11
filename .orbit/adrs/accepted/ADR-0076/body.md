## Context
Three benchmark rounds asked whether the eight-tool agent-facing `orbit_graph_*` MCP surface earns its token cost against grep/read. v1/v2 first looked like a null result, but codex only had shell access to graph tools there; v3 gave codex MCP parity and hybrid utilization moved from 0/30 to 23/30, with hybrid aggregate tokens at 0.65× no-graph and graph-only accuracy at 30/30. Claude stayed at 0/30 hybrid graph use because its baseline already included specialized `Read` / `Grep` / `Glob`. The pre-registered keep threshold passed on utilization but remained mixed on per-cell cost: codex passed 4/10 fixtures, claude 1/10.

## Decision
Retain the agent-facing `orbit_graph_*` MCP surface. The decisive signal is provider-specific utilization when the graph tools are first-class; the cost signal is useful but not a clean pass. The duplicated top-level evidence log was folded into this ADR and removed under [T20260430-22].

## Consequences
- The eight-tool `orbit_graph_*` MCP surface stays shipped.
- A diagnostic v4 round is planned to characterize the cost-overshoot fixtures, not to re-litigate keep/cull. Targets: the `impact-tool-context-struct-literals` firehose (12.43× on codex), the signature-vs-type-resolved precision gap, and payload-volume problems on `pack`-heavy navigations.
- Future work on schema-cache overhead and payload size (pointer-only graph reads, [T20260423-0607]) is a measured-need item, not speculative.
- Provider-dependent caveat: the surface earns its cost where the baseline tool list is generic (codex's `exec_command`-only). On providers whose baseline already includes specialized fs primitives that overlap in function (Claude), the data is consistent with "graph tools exist but don't get used" — a latent schema-cache tax paid without return. Whether eating that tax to keep codex happy is worth it is a product question, not a benchmark question.
- Future tool-surface decisions for other specialized orbit tooling should examine the same question: is the new tool competing in a shell selector (win), a tool-list selector against a generic alternative (win), or a tool-list selector against a specialized alternative (likely loss).
- Future benchmark thresholds must specify both a per-cell threshold and the aggregation rule before a sweep runs.
- Cost: the MCP schema payload and prompt budget tax remain provider-dependent; retaining the surface deliberately accepts that overhead for providers that do use it.

---
