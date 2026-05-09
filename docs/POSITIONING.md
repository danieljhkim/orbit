# Orbit Positioning

This document names what Orbit is for, who it's for, and how the audience expands over time. Use it as a decision lens — when a design debate feels like it's really about *what Orbit is for*, reference this doc instead of re-litigating case by case.

## What Orbit is for

**A durable, intent-tracked, auditable task layer for developers driving AI coding agents at high volume — local-first by design, with a path to team-scale automation as trust in agents matures.**

The wedge today is the individual engineer driving multiple coding agents against real code and needing the work to outlive any single agent session: a persistent backlog, an audited execution trail, and intent attribution at the codebase level — every line of agent-authored code traceable to the task that produced it. Agent vendors solve in-session execution; nobody else has solved the durable, intent-tracked, audited layer above for the AI-native solo developer who plans to expand it across their team. Orbit does.

The destination — once trust in agents matures — is fleet orchestration at team scale, served by Orbit's hosted team product (a separate paid SKU). Multi-year arc; the wedge does not depend on its timeline.

## Commercial model: open-core, two tiers

Architecture-level split (separate repositories), not feature flags.

- **Orbit OSS.** Self-hosted, single-operator. Agent loop, knowledge graph, audit, task layer, MCP, providers, CLI. MIT/Apache 2.0. Free forever for individuals and small teams self-hosting. Self-sufficient — no single-operator workflow is gated behind the paid tier.
- **Orbit Team** (in development). Hosted multi-tenant for engineering organizations. Cross-engineer audit aggregation, team scoreboards, SSO/SAML, RBAC, hosted ops, support SLAs. Closed-source SaaS, separate repository, separate billing.

Boundary rule: *"would a solo developer running self-hosted Orbit on their laptop want this?"* — yes → OSS, no → Team. Apply consistently and the boundary doesn't drift.

## Who Orbit is for, in funnel order

Three stages of one audience:

1. **Wedge — AI-native solo developers** running multiple coding agents (Claude Code, Cursor, Aider, Codex CLI) heavily, who have outgrown the in-session model.
2. **Champion — staff/principal engineers, tech leads, founding engineers** with organizational influence. Validate on personal work, then advocate internally.
3. **Destination — team-scale agentic automation via Orbit Team.** Target segment: growth-stage and mid-market (10–500 engineers). Fortune-500 enterprise is multi-year-out and not a near-term target.

We optimize the OSS for stage 1. Stage 3 conversion happens because individual engineers carry Orbit into their teams, not because Orbit pitches teams directly.

## What Orbit is NOT for

- **Enterprise surface bolted onto OSS.** SOC 2, SSO/SAML, RBAC, 24/7 support belong in Orbit Team. Demand served via the separate SKU, not by polluting OSS.
- **Generic workflow orchestration.** n8n, Airflow, LangGraph, Temporal — Orbit is a coding-agent platform, not a workflow engine.
- **Hidden cloud dependencies.** Orbit must never phone home.
- **Vendor lock-in to one LLM provider.** Cross-provider is table stakes.
- **Black-box agent decisions.** Every agent decision should be inspectable.
- **Onboarding designed for non-technical users** — patronizing and misaligned with the audience.
- **Subscription-arbitrage architectures** that assume a single personal CLI account is the backbone. The wedge drives multiple agents in parallel against real code; that breaks the per-account assumption.

## Primary focus: auditability

Auditability is a product feature, not a cross-cutting concern. When something goes wrong, the operator answers *what / why / who* without calling the maintainers.

- **Complete coverage** — every operation that touches code, state, or external services emits an audit event. Silent paths are bugs.
- **Structured, queryable events** — typed records with stable schemas, exportable to your own observability stack.
- **Faithful reproducibility** — prompts and responses stored verbatim (configurable redaction). Summaries are derived, not replacements.
- **Tamper-evident retention** — append-only, verifiable.
- **Agent-identity attribution** — every write carries the identity of the agent (and model) that produced it.

When auditability conflicts with performance, ergonomics, or feature surface, auditability wins.

## Non-negotiables

- **Self-hostable OSS under permissive license.** Single binary, no mandatory cloud dependency.
- **Open-core split, architectural not feature-flag.** Separate repos, separate licenses.
- **Bring-your-own-credentials.** API keys belong to the operator; Orbit is pass-through.
- **HTTP/SDK-first provider communication.** CLI shell-out is an escape hatch, not the backbone.
- **Audit trail for everything that touches code.** See above.
- **Intent attribution at the codebase level.** `task_id` in commit messages, queryable, durable across rewrites.
- **Reproducibility where possible, recorded non-determinism where not.**
- **Knowledge-graph–aware tooling.** Agents query a parsed, symbol-level graph. The graph is what makes audit cheap to populate; benchmark validation in `benchmarks/graph/`.
- **Cost-visible.** Operator knows what each run costs in tokens and wall-clock.
- **Git- and GitHub-native.** No custom VCS abstractions.
- **Configurable, not rigidly opinionated.** Job DAGs, activities, skills, role profiles are YAML data, not code.

`task_id` is locally meaningful by design — a personal search key for the task author, recorded in local audit. Not resolvable on another engineer's machine; for cross-engineer references, use `external_refs` to link tasks to your team's tracker (Jira, Linear, GitHub Issues, etc.).

## Commercial roadmap: Orbit Team

What lives in Team (NOT OSS):

- **Hosted multi-tenancy.** One Orbit Team instance per organization.
- **Cross-engineer audit aggregation.** Per-operator primitives ship in OSS; cross-operator aggregation, query API, and team-wide UI ship in Team.
- **Team-grade fleet metrics.** Cross-engineer throughput, team-wide PR merge rates, multi-operator policy enforcement.
- **Organizational governance.** SSO/SAML, RBAC, audit retention, compliance attestations.

What lives in OSS (not Team-only): per-operator fleet primitives. The AI-native solo developer drives multiple agents in parallel; OSS treats fleets as the default execution shape on a single host.

**GTM motion.** Champion-led, bottom-up. Pricing per-seat hosted; self-serve for small teams, sales-assisted for larger deployments.

## The decision lens

When a design decision is contested, the tiebreaker:

> **Would this hold up for an AI-native solo developer driving multiple agents against real code, AND would it still hold up at team scale once trust matures?**

If the honest answer to either half is no, it doesn't ship.

Secondary lenses, in order:

1. **Does this serve the wedge as much as the destination?** Features that only make sense at team scale shouldn't ship until the audience catches up.
2. **Would this still be true at 10× the current agent fleet size?**
3. **Can the operator debug this without asking us?** If not, the feature is under-instrumented.
4. **Does this survive losing confidence in a single provider?**

## Boundaries (when we'd reconsider)

This positioning is not permanent. Reconsider if:

- 90-day distribution experiments produce zero substantive replies, trial users, or traction.
- 12+ months OSS adoption with zero teams committing to evaluate Orbit Team.
- True Fortune-500 enterprise demand materializes faster than Team is ready.
- Trust in agents matures dramatically faster than expected, collapsing the multi-year arc.
- The team-scale destination stops being a coherent niche.
- Hosted Team launches and customer feedback says the open-core boundary is in the wrong place — relocate features; do not blur the architectural split.

Until one of those happens, the framing above is the lens.
