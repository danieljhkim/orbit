# Agent Families — Decisions

**Status:** Draft
**Owner:** grok
**Last updated:** 2026-05-16

ADR entries are append-only and ordered ascending.

## ADR-0151 (2026-05-16)

**Title:** Add Grok (xAI) as a fourth peer agent family

**Decision:** Treat "grok" as a full peer alongside claude, codex, and gemini.

**Key Changes:**
- Extended `agent_from_model()`, `infer_agent_family_from_model()`, `all_agent_families()`, `resolve_agent_model_pair()`, and `provider_from_model()`
- Added `grok.yaml` executor skeleton and sandbox support (tasks ORB-00044, ORB-00045)
- Added Grok provider to `orbit mcp init` (ORB-00046)
- Created this design doc folder (ORB-00052)

See full ADR-0151 for context, alternatives considered, and cost analysis.

## Task References

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
