## Context
[ORB-00154] found that the dashboard Scoreboard fragmented the four canonical agents across six stacked tables, repeated headers, sparse zero glyphs, and bare integers. Real alternatives included keeping grouped tables, switching to an agent-major wide table, using per-agent cards, or reducing the view to a pure heatmap.

## Decision
Render canonical scoreboard metrics as one metric-major Unified Leaderboard Matrix: metric rows grouped by Delivery, Review, Knowledge, Operations, and Planning Duels, with codex, claude, gemini, and grok as fixed columns. Non-zero metric cells carry inline bars scaled within the metric row, tied leaders get an explicit leader badge, zero values render as an em dash instead of a visible zero glyph, the Duel Matrix remains compact below, and Attribution Cleanup renders only when non-canonical agents have non-zero signal.

## Consequences
- Operators can identify the leading agent per metric by bar length and leader badge without comparing digit strings across repeated tables.
- The canonical agent set remains the primary row population while non-canonical attribution stays conditional.
- Rejected alternative: agent-major flat wide table; at roughly twenty metric columns it would require horizontal scrolling at the canonical dashboard viewport.
- Rejected alternative: per-agent card grid; it preserves the need to scan separate blocks to answer which agent leads a specific metric.
- Rejected alternative: pure heatmap matrix; color alone hides precise values needed for operator judgment.
- No single Rust code anchor; this UI convention is enforced in dashboard rendering and design review, and workspace-local ADR comments are not embedded in shipped dashboard assets.
- Cost: the matrix is denser and needs careful row-height discipline when new metrics are added.