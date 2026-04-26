# Orbit Positioning

This document names who Orbit is for, what we build for them, and what we don't.

Use it as a decision lens. When a design debate feels like it's really about *who Orbit is for*, reference this doc instead of re-litigating case by case.

## Who Orbit is for

**The staff engineer or platform lead at a team of ~10–50 engineers who wants to run agent automation against their team's real codebase.**

They are not writing an agent for a demo. They are not a solo developer augmenting their own workflow. They are not a Fortune 500 buyer looking for a vendor. They are an engineer with authority over their team's tooling who has decided that LLM-driven automation — PR review, refactor passes, backlog execution, cross-cutting migrations — is worth running at team scale, and wants infrastructure they can actually rely on.

Concretely, they:

- Self-host OSS. They will not route their source through a third-party SaaS.
- Have API credits — either personal, their org's Anthropic contract, or local model budget (GPUs, subscriptions irrelevant).
- Run Orbit against a real production monorepo with real reviewers, real CI, and real consequences when an agent merges something wrong.
- Need audit trails, reproducibility, and observability — not because compliance requires it, but because they have to explain what happened when something goes sideways.
- Are technical enough to configure, debug, and extend. They reject configuration designed for non-technical users.
- Want fleets — multiple agents, possibly multiple providers, running in parallel — not a single chat assistant.

## Who Orbit is NOT for

Three audiences we are deliberately not serving. Each would pull Orbit in a direction that makes it worse for the primary audience.

**1. Individual developers augmenting personal workflow.** This audience is already served well by Claude Code, Cursor, Aider, Codex CLI. Optimizing for them — subscription-backed backends, zero-config installs, personal-context assumptions — compromises the properties the primary audience needs (throughput, fleet behavior, clean API accounting).

**2. Enterprise procurement buyers.** SOC 2, SSO, multi-tenant permissions, 24/7 support contracts, sales motions, long procurement cycles. Serving this audience requires headcount, compliance work, and a product management discipline that are incompatible with how Orbit is built. If enterprise demand appears in volume, the honest path is a commercial fork, a partnership, or an acquisition — not bolting enterprise surface onto the OSS core.

**3. Generic workflow orchestrators.** n8n, Airflow, LangGraph, Temporal. Orbit is specifically a coding-agent platform, not a generic workflow engine. Features that are valuable only in non-coding contexts belong elsewhere.

## Primary focus: auditability

Auditability is not a cross-cutting concern in Orbit — it is a product feature. The primary audience runs agents against code their team has to live with. When something goes wrong (a bad merge, a regression, a mystery refactor), they need to answer three questions without calling the Orbit maintainers:

1. **What did the agent do, exactly?** Every tool call, every provider request/response, every task state transition recorded with enough fidelity to reconstruct the sequence.
2. **Why did it do that?** The prompt, system instructions, role configuration, and surrounding context are recoverable, not just the action.
3. **Who is accountable?** Agent identity, model, provider, and activity context are attached to every event — on commits, on PRs, on audit entries — so a git blame or audit query reaches a concrete agent identity, not "the AI."

Concrete commitments this implies:

- **Complete coverage.** Every operation that touches code, state, or external services emits an audit event. Silent paths are bugs.
- **Structured, queryable events.** Not log strings. Typed records with stable schemas the user can query with `orbit.audit.*` tools and export to their own observability stack.
- **Faithful reproducibility.** Prompts and responses are stored verbatim (with configurable redaction for sensitive paths). Summaries are derived artifacts, not replacements.
- **Tamper-evident retention.** Audit is append-only. The audit trail's own integrity is verifiable; corrupting history should not be a silent operation.
- **Agent-identity attribution.** Every write — commit, PR, audit event, task update — carries the identity of the agent (and model) that produced it. No anonymous AI actions.

When auditability conflicts with performance, ergonomics, or feature surface, auditability wins. Undercutting audit fidelity to save a tool call or a storage row is the kind of decision this doc exists to prevent.

## What this audience needs (non-negotiables)

- **Self-hostable.** Single binary, no mandatory cloud dependency. Runs on a laptop, in a container, in a CI runner, behind a firewall.
- **Bring-your-own-credentials.** Orbit never stands between the user and the provider. API keys are the user's; Orbit is a pass-through.
- **HTTP/SDK-first provider communication.** Programmatic multi-turn is what this audience is doing. CLI shell-out is an escape hatch for experimentation, not the backbone.
- **Audit trail for everything that touches code.** See the dedicated *Primary focus: auditability* section above. Non-negotiable and promoted to a product feature, not a compliance concession.
- **Reproducibility where possible, recorded non-determinism where not.** Same task + same repo state should converge. When the provider introduces non-determinism, capture it rather than hide it.
- **Fleet primitives.** Parallel task execution, cross-provider delegation, per-agent scoreboards, per-agent identity in commits. Single-assistant assumptions are incorrect.
- **Knowledge-graph–aware tooling.** Agents operate against a parsed graph of the codebase, not raw grep. This is the technical moat and the reason to pick Orbit over a generic agent framework.
- **Cost-visible.** The user knows what each run costs in tokens and wall-clock.
- **Git- and GitHub-native.** Branches, worktrees, PRs, CI status. No custom version control abstractions.
- **Configurable, not opinionated to the point of rigidity.** Job DAGs, activity definitions, skill loadouts, role profiles are all data (YAML), not code. The primary audience will fork, not file feature requests.

## What this audience rejects

- Hidden cloud dependencies. If Orbit phones home, the audience leaves.
- Vendor lock-in to one LLM provider. Cross-provider is table-stakes.
- Magic. If the audience can't figure out why an agent did something, trust erodes permanently.
- Enterprise surface bolted onto OSS core (tenancy, ACLs, SSO) — adds complexity without serving them.
- Onboarding designed for non-technical users — patronizing and misaligned with how they work.
- Subscription-arbitrage architectures that assume a personal CLI account is the backbone. They will run Orbit at scales that break that assumption.
- Black-box "just trust the agent" patterns. Every agent decision should be inspectable.

## The decision lens

When a design decision is contested, the tiebreaker is:

> **Would a staff engineer running Orbit against their team's production monorepo rely on this?**

If the honest answer is no, it doesn't ship — even if it serves individual developers or looks good in a demo.

Secondary lenses, in order:

1. **Would this still be true at 10× the current agent fleet size?** If a pattern only works with one orchestrator and three agents, it's wrong.
2. **Can the user debug this without asking us?** If the answer requires Orbit maintainers in the loop, the feature is under-instrumented.
3. **Does this survive the user losing confidence in a single provider?** If a design fails when Anthropic rate-limits or deprecates a model, it fails.

## Boundaries (when we'd reconsider)

This positioning is not permanent. We'd reconsider if:

- Enterprise demand arrives with headcount or funding to serve it properly. The honest response is a commercial arm or partner, not a quiet pivot of the OSS core.
- Individual-developer demand reaches a scale where a separate "Orbit Lite" makes sense. The honest response is a separate product, not feature-bloat on the core.
- The primary audience itself changes (e.g., the "staff eng at 3–50 person team" archetype stops being the natural fit). Revisit the whole document in that case — don't patch around it.

Until one of those happens, the audience above is the lens.
