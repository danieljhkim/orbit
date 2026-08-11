---
title: <Feature> — Decisions
owner: <agent family: codex | claude | grok | gemini>
last_updated: YYYY-MM-DD
last_validated: YYYY-MM-DD
status: Draft
feature: <feature-slug>
doc_role: decisions
type: design
summary: <one-line hook for agent retrieval — non-empty, single line>
tags: [<feature-slug>]
paths: ["crates/<crate>/**"]
related_features: [<feature-slug>]
related_artifacts: [ADR-NNNN]
---

# <Feature> — Decisions

Decision record for <feature>, in ascending number order. This file is the
authoritative body — there is no ADR store behind it. Numbering is repo-local:
take the next unused number with `grep -rho 'ADR-[0-9]\{4\}' docs/ | sort -u | tail -1`.

An entry is admitted through exactly one of two doors: it explains a specific
code site that would otherwise look wrong (Door 1, carries `code_anchors:`), or
it states a standing rule that decides future tradeoffs (Door 2, carries
`scope:`). Everything else is design prose and belongs in `2_design.md`. See
[CONVENTIONS.md §4](../CONVENTIONS.md#4-adrs-strict) for the full rules,
including the mandatory `Cost:` line, supersession, and rollups.

<!-- Copy ONE of the two blocks below for each new ADR. Delete this comment in real docs. -->

## ADR-NNNN — <short title, noun phrase>

**Status:** Accepted · YYYY-MM · [ORB-NNNNN]
**Code anchors:** `crates/<crate>/src/<file>.rs::<symbol>`

### Context

<What forced a decision. The constraint, not the narrative.>

### Decision

<What was chosen, stated so a reader can act on it.>

### Consequences

- <What is now true.>
- Cost: <what this gives up — something a reader could not infer from the decision itself.>

## ADR-NNNN — <short title, noun phrase>

**Status:** Accepted · YYYY-MM · [ORB-NNNNN]
**Scope:** <the areas this rule governs>

### Context

<The recurring tradeoff this settles.>

### Decision

<The standing rule, phrased so it applies to a case nobody has seen yet.>

### Consequences

- <What this rules out.>
- Cost: <what the rule costs when it binds.>

## Task References

- [ORB-NNNNN] — <verb phrase: what the task did>

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
