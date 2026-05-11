# ADR Artifact — Vision

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-10

This document captures open questions that the v2 implementation should resolve, prior art that informed the proposal, and what — if anything — is distinctive about doing ADRs this way inside an agent-first system. See [1_overview.md](./1_overview.md) for motivation and [2_design.md](./2_design.md) for the mechanism.

---

## 1. Open Questions

### 1.1 Retire `4_decisions.md`, or auto-generate?

**Resolved by [ADR-0020](./4_decisions.md#adr-006--auto-generate-per-feature-4_decisionsmd-index).** Auto-generate from the store; `4_decisions.md` becomes a build artifact committed to git for reviewers without Orbit installed.

### 1.2 Should ADR scope extend beyond feature folders?

**Resolved by [ADR-0021](./4_decisions.md#adr-007--cross-cutting-adrs-use-a-dedicated-cross-cutting-index).** Yes. Cross-cutting ADRs live in a dedicated `docs/design/cross-cutting/` pseudo-feature. CLAUDE.md rules that earn ADR status migrate there over time. The follow-up sweep — identifying which existing CLAUDE.md rules should become ADRs — is its own task.

### 1.3 Should ADR creation block on task linkage?

**Resolved by [ADR-0022](./4_decisions.md#adr-008--adr-creation-does-not-require-task-linkage).** No. Empty `related_tasks` at creation is permitted; the task requirement applies only at the `proposed → accepted` transition. The decision is explicitly framed as "iterate before constraining" — most likely candidate for revision based on observed corpus behavior.

### 1.4 Markdown body vs. structured sub-fields

The current design keeps Context / Decision / Consequences as one markdown body (see [ADR-0018](./4_decisions.md#adr-004--markdown-body-structured-envelope)). Alternative: split each into a structured field, parseable for review (e.g., *"show me every ADR whose Cost line mentions latency"*).

Splitting enables better queries; it also imposes a writing burden that today's markdown template doesn't. Re-evaluate once the corpus is large enough to know what queries would actually be useful. Remains open.

### 1.5 Review workflow

**Resolved by [ADR-0023](./4_decisions.md#adr-009--review-threads-on-adrs).** ADRs get `orbit.adr.review_thread.{add, list, reply, resolve}`, mirroring the task surface. Whether the `proposed → accepted` transition should require all threads resolved is a sub-question deferred until the surface has real use.

### 1.6 Primary feature for cross-cutting ADRs

**Resolved by [ADR-0021](./4_decisions.md#adr-007--cross-cutting-adrs-use-a-dedicated-cross-cutting-index)** (combined with §1.2). A cross-cutting ADR's "primary" folder is `cross-cutting/`. Per-feature indexes still include cross-cutting ADRs that touch them via the existing `--feature` filter against `related_features`.

---

## 2. Prior Work

### 2.1 ADRs as markdown — the prevailing pattern

Michael Nygard's 2011 post (*Documenting Architecture Decisions*) established the template the Orbit folders use today: short, append-only, one decision per file or section, with Context / Decision / Consequences. ThoughtWorks codified it as a Tech Radar item; Spotify's ADR repository is the canonical large-scale example. All of these store ADRs as markdown — directly committed, no structured store.

What works in that model: low ceremony, full grep, version-controlled. What breaks: the same problems §1 of [1_overview.md](./1_overview.md) names — no index, no cross-reference, no lifecycle enforcement at scale.

### 2.2 Structured RFCs

The IETF RFC process and its descendants (Rust RFCs, Python PEPs) push the other direction: every decision is a structured document with metadata, a numbered ID, a discussion thread, and explicit status (`Draft / Active / Withdrawn / Superseded`). The cost is heavy ceremony — a Rust RFC takes weeks. ADR-as-artifact sits between Nygard-style markdown and RFC-style ceremony: structured metadata, lightweight body, agent-creatable in one tool call.

### 2.3 Issue trackers as decision stores

Some teams use Jira / Linear / GitHub Issues as the ADR record (label = `adr`, status maps to issue state). It solves indexing and search but loses the *colocated-with-code* property that markdown ADRs have. ADR-as-artifact preserves colocation by storing in `.orbit/adrs/` next to the workspace it governs, while gaining the indexing.

### 2.4 Orbit-internal precedent

Tasks themselves followed this trajectory. Early Orbit had tasks as ad-hoc markdown notes; the current `orbit-store::task_store` evolved them into structured artifacts with status directories, SQLite index, and a tool surface. ADRs are a near-identical refactor against the same forces — the design literally mirrors `task_store` (see [2_design.md §3](./2_design.md)).

---

## 3. What May Be Distinctive

Most ADR systems are read by humans, on demand, in a code review or onboarding. ADRs in Orbit are read primarily by **agents**, every time a task touches a feature. That changes which properties matter:

- **Cheap lookup beats browsability.** A human skimming `4_decisions.md` reads ten ADRs they don't need. An agent calling `orbit.adr.show ADR-0042` pays for exactly one. Agent context budgets make this asymmetry sharper than it is for humans.
- **Lifecycle enforcement at write time, not review time.** Humans flipping `Proposed → Accepted` in markdown is error-prone; the transition rule (must cite a shipped task) is enforceable when agents make the change via `orbit.adr.update`.
- **Semantic search becomes load-bearing.** Most ADR systems treat search as a nice-to-have because humans navigate by folder. Agents don't browse folders — they query. `orbit.adr.search "sandbox-exec wrapper"` is the primary access pattern, not a fallback.

None of this is novel in isolation. The combination — structured ADRs + agent-primary readers + tool-surfaced lifecycle — may be.

---

## 4. References

### Orbit-internal

- [docs/design/CONVENTIONS.md](../CONVENTIONS.md) — current ADR template and folder conventions this proposal modifies.
- [docs/design/task-sync/](../task-sync/) — closest v2-design analog; also docs-only at v1.
- [crates/orbit-store/src/file/task_store/](../../../crates/orbit-store/src/file/task_store/) — the layered-store pattern this design copies.
- [crates/orbit-embed/src/vector/](../../../crates/orbit-embed/src/vector/) — semantic-search infrastructure reused by `orbit.adr.search`.

### External

- Nygard, *Documenting Architecture Decisions* (2011).
- ThoughtWorks, *Lightweight Architecture Decision Records* (Tech Radar, 2017).
- IETF RFC 7322, *RFC Style Guide* — the heavyweight pole of the spectrum.
- adr-tools (Nat Pryce) — file-based ADR CLI; informed our tool naming.

---

## Task References

- [T20260510-27] — Drafted the adr-artifact design folder as a v2 proposal. This document captures open questions, prior work, and what may be distinctive about doing ADRs this way inside an agent-first system.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
