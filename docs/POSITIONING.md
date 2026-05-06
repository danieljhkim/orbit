# Orbit Positioning

This document names what Orbit is for, what we build for, and what we don't.

Use it as a decision lens. When a design debate feels like it's really about *what Orbit is for*, reference this doc instead of re-litigating case by case.

## What Orbit is for

**Running coding-agent automation against a team's real production codebase. v1 is per-operator: each member of a 10–50 engineer team runs Orbit on their own machine, coordinating through the team's existing git and GitHub primitives. v2 commits to a shared-host deployment shape that aggregates audit, tasks, and scoreboards across the team.**

Not a demo. Not a solo-developer productivity boost. Not a Fortune-500 vendor pitch. The work in scope is PR review, refactor passes, backlog execution, and cross-cutting migrations against code that has reviewers, CI, and real consequences when an agent merges something wrong.

The deployment shape that fits:

- **Self-hosted OSS.** Source does not route through a third-party SaaS.
- **Bring-your-own credentials.** Anthropic / OpenAI contracts, local model budget — never Orbit's keys.
- **Real production monorepo.** Real reviewers, real CI, real consequences for bad merges.
- **Audit trails, reproducibility, observability** as first-class requirements — not because compliance dictates, but because someone has to explain what happened when an agent goes sideways.
- **Technical configuration surface.** Readable, debuggable, forkable YAML — not a UX patronizing toward non-technical users.
- **Fleets, not a single assistant.** Multiple agents, possibly multiple providers, running in parallel.
- **Per-operator install.** Orbit runs on each engineer's machine, not on a shared server. Provider credentials, audit DB, locks, and the dashboard are local to the operator. Cross-engineer coordination flows through git and GitHub.

Orbit `task_id` is locally meaningful. It appears in commits as a personal search key for the task author and is recorded in local audit. It is not designed to be resolvable on another engineer's machine; for cross-engineer task references, use `external_refs` to link tasks to your team's tracker (Jira, Linear, GitHub Issues, etc.).

## What Orbit is NOT for

Three use cases we are deliberately not serving. Each would pull Orbit in a direction that makes it worse for the in-scope use case.

**1. Individual developers augmenting personal workflow.** Already well served by Claude Code, Cursor, Aider, Codex CLI. Optimizing for that shape — subscription-backed backends, zero-config installs, personal-context assumptions — compromises the properties team-scale deployment needs (throughput, fleet behavior, clean API accounting).

**2. Enterprise procurement.** SOC 2, SSO, multi-tenant permissions, 24/7 support contracts, sales motions, long procurement cycles. Serving these requires headcount, compliance work, and product management discipline incompatible with how Orbit is built. If enterprise demand appears in volume, the honest path is a commercial fork, a partnership, or an acquisition — not bolting enterprise surface onto the OSS core.

**3. Generic workflow orchestration.** n8n, Airflow, LangGraph, Temporal. Orbit is specifically a coding-agent platform, not a generic workflow engine. Features that are valuable only in non-coding contexts belong elsewhere.

## Primary focus: auditability

Auditability is not a cross-cutting concern in Orbit — it is a product feature. Orbit runs agents against code that a team has to live with. When something goes wrong (a bad merge, a regression, a mystery refactor), the operator needs to answer three questions without calling the Orbit maintainers:

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

## Non-negotiables

- **Self-hostable.** Single binary, no mandatory cloud dependency. Runs on a laptop, in a container, in a CI runner, behind a firewall.
- **Bring-your-own-credentials.** Orbit never stands between operator and provider. API keys belong to the operator; Orbit is a pass-through.
- **HTTP/SDK-first provider communication.** Programmatic multi-turn is the deployment shape. CLI shell-out is an escape hatch for experimentation, not the backbone.
- **Audit trail for everything that touches code.** See the dedicated *Primary focus: auditability* section above. Non-negotiable and promoted to a product feature, not a compliance concession.
- **Reproducibility where possible, recorded non-determinism where not.** Same task + same repo state should converge. When the provider introduces non-determinism, capture it rather than hide it.
- **Fleet primitives, per operator.** Parallel task execution, cross-provider delegation, per-agent scoreboards, per-agent identity in commits — across many agents on one operator's machine. Single-assistant assumptions are incorrect. Shared-multi-operator assumptions are also incorrect for v1.
- **Knowledge-graph–aware tooling.** Agents operate against a parsed graph of the codebase, not raw grep. This is the technical moat and the reason to pick Orbit over a generic agent framework.
- **Cost-visible.** The operator knows what each run costs in tokens and wall-clock.
- **Git- and GitHub-native.** Branches, worktrees, PRs, CI status. No custom version control abstractions.
- **Configurable, not opinionated to the point of rigidity.** Job DAGs, activity definitions, skill loadouts, role profiles are all data (YAML), not code. Forking is expected; feature requests for narrow customization are not.

## What Orbit refuses to add

- Hidden cloud dependencies. Orbit must never phone home.
- Vendor lock-in to one LLM provider. Cross-provider is table-stakes.
- Magic. If an agent decision can't be reconstructed, trust erodes permanently.
- Enterprise surface bolted onto OSS core (tenancy, ACLs, SSO) — adds complexity without serving the in-scope use case.
- Onboarding designed for non-technical users — patronizing and misaligned with the deployment shape.
- Subscription-arbitrage architectures that assume a personal CLI account is the backbone. Team-scale workloads break that assumption.
- Black-box "just trust the agent" patterns. Every agent decision should be inspectable.

## The decision lens

When a design decision is contested, the tiebreaker is:

> **Would this hold up against a real production monorepo with real reviewers, real CI, and real consequences for a bad merge?**

If the honest answer is no, it doesn't ship — even if it serves individual-developer use cases or looks good in a demo.

Secondary lenses, in order:

1. **Would this still be true at 10× the current agent fleet size?** If a pattern only works with one orchestrator and three agents, it's wrong.
2. **Can the operator debug this without asking us?** If the answer requires Orbit maintainers in the loop, the feature is under-instrumented.
3. **Does this survive losing confidence in a single provider?** If a design fails when Anthropic rate-limits or deprecates a model, it fails.

## Boundaries (when we'd reconsider)

This positioning is not permanent. We'd reconsider if:

- Enterprise demand arrives with headcount or funding to serve it properly. The honest response is a commercial arm or partner, not a quiet pivot of the OSS core.
- Individual-developer demand reaches a scale where a separate "Orbit Lite" makes sense. The honest response is a separate product, not feature-bloat on the core.
- The team-scale deployment context itself stops being a coherent niche (e.g., team-scale agent automation collapses into solo tooling above and enterprise platforms below). Revisit the whole document in that case — don't patch around it.
- The shared-host deployment shape (one Orbit serving multiple operators) ships in v2 and demand for it materializes. At that point the per-operator framing in this doc narrows from "what Orbit is" to "what v1 was" — reframe rather than maintain stale per-operator language once shared-host deployment lands.

Until one of those happens, the framing above is the lens.
