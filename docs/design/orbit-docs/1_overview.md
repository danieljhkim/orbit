---
title: "Orbit Docs — Overview"
owner: claude
last_updated: 2026-05-21
status: Draft
feature: orbit-docs
doc_role: overview
type: design
summary: "Orbit Docs — what the human-authored docs corpus is, why it exists alongside learnings and ADRs, and how agents retrieve from it."
tags: [orbit-docs]
related_features: [orbit-docs]
related_artifacts: [ORB-00163, ORB-00206, ADR-0169, ADR-0170, ADR-0171, ADR-0180]
---

# Orbit Docs — Overview

Orbit Docs is the human-authored knowledge corpus for an Orbit workspace. It indexes the Markdown a team writes for itself — design narratives, reusable code patterns, runbooks, glossaries — and exposes a single retrieval surface (`orbit.docs.*`) that agents can query at task time, at hook time, or interactively. It deliberately does not own the corpus's storage shape: docs are PR-reviewed files under a configurable `docs/` root, and Orbit's only on-disk artifact is the `[docs].roots` entry in `.orbit/config.toml`.

The system is **pull-first**: agents call `orbit search --kind doc` (or `--kind all` for federated doc+ADR) or `orbit docs show` when they need context. Push-style injection (PreToolUse hook surfaces, `task show --with-context`) is a downstream feature, designed but not yet wired ([ORB-00166], [ORB-00167]).

Phase 1 ships the corpus, the locked frontmatter schema, the six-verb surface, the `orbit-docs` skill, doc-corpus embeddings via `orbit docs index`, and a one-shot migrator that backfills legacy `docs/design/<feature>/` and `docs/design-patterns/` files. [2_design.md](./2_design.md) specifies the schema, walker, surface, tolerant indexer, and hybrid search path; [3_vision.md](./3_vision.md) names open questions and the remaining roadmap (ADR folding, hook integration); [4_decisions.md](./4_decisions.md) is the ADR log.

---

## 1. Motivation

Three concrete gaps existed before [ORB-00163]:

1. **Learnings cover load-bearing micro-rules — not explanatory context.** Learnings are scope-globbed, supersedable, and CRUD'd through `orbit.learning.*`. They were designed to carry *rules with known failure modes*, not multi-page design narratives. Stretching them to cover designs would distort the data model. See [docs/design/project-learnings/](../project-learnings/) for the learning shape.
2. **Design docs were over-enforced.** The older `orbit-design` skill enforces a strict 4-numbered-doc layout under `docs/design/<feature>/`, a `Last updated:` freshness rule, and the ADR earning rule. Two of those three (layout + freshness) are over-opinionated for a framework-layer tool that's supposed to compose with team conventions. The ADR rule belongs to `orbit-adr`, not to a docs skill. Retiring `orbit-design` is filed as [ORB-00165].
3. **Other knowledge categories had no indexed surface.** Reusable code patterns (`docs/design-patterns/`), operational runbooks, business/domain context, glossaries — all existed in the repo but had no retrieval primitive. Agents could grep, but they had no way to ask "what's the documented shape for crate-boundary error translation?" without already knowing the file path.

The hard constraint that shaped the design: **the corpus has to be tolerant.** Existing `docs/design/<feature>/*.md` and `docs/design-patterns/*.md` files have no frontmatter, and we will not force a flag-day migration. The indexer infers `type` and `summary` from directory and filename heuristics when frontmatter is absent, so day-one retrieval works without any author effort. The `migrate` verb provides the optional one-shot backfill.

A second constraint: **no enforcement.** Orbit Docs does not require the 4-numbered layout, the `Last updated:` line, or any specific section structure. It indexes whatever is under configured roots with valid frontmatter (strict mode) or any Markdown at all (tolerant mode). Team conventions stay where they belong — in `docs/design/CONVENTIONS.md` if the team writes one — and Orbit Docs neither enforces nor contradicts them.

---

## 2. Core Concepts

### 2.1 Doc

A `.md` file under a configured `[docs].roots` path, with optional locked frontmatter. The body is the Markdown after the frontmatter block. Docs have no Orbit-allocated ID; they are referenced by repo-relative path.

### 2.2 Frontmatter (locked)

Six fields, two required, four optional:

| Field | Required | Shape | Purpose |
|-------|----------|-------|---------|
| `type` | yes | enum: `design \| pattern \| context \| glossary \| runbook` | Coarse classifier for filtering. |
| `summary` | yes | non-empty single line | One-line retrieval hook; what the doc is about. |
| `tags` | no | string list | Free-form labels; used by `orbit docs list --tag`. |
| `paths` | no | glob string list | File-scope patterns this doc applies to (e.g. `crates/orbit-cli/**`). Used by hook-time injection. |
| `related_features` | no | string list | Feature slugs this doc covers; join key with task `related_features`. |
| `related_artifacts` | no | string list | Cross-references to other Orbit artifacts via [ADR-0171] ID-prefix dispatch. |

Schema rationale and the closed-by-default choice: [ADR-0169]. Why ID-prefix dispatch over object-shape references: [ADR-0171].

### 2.3 Tolerant indexer

Files without frontmatter, or with malformed frontmatter, are not silently dropped. The walker falls back to:

- `type`: inferred from directory (`docs/design/<feature>/` → `design`, `docs/design-patterns/` → `pattern`, dir containing `runbooks` → `runbook`, filename or dir matching `glossary` → `glossary`, otherwise `context`).
- `summary`: the first non-empty non-frontmatter Markdown line after stripping `#` heading markers; falls back to a titleized filename stem.
- `tags`: feature slug for design docs (e.g. `tags: [activity-job]` for `docs/design/activity-job/...`); empty otherwise.

Strict parsing still applies if you opt in via the `migrate` verb or by writing frontmatter manually. Tolerant fallback exists so the corpus is queryable on day one without any flag-day work.

### 2.4 Six-verb surface

| Verb | Purpose |
|------|---------|
| `orbit docs list` | Walk configured roots; return all records (with optional `--type` and `--tag` filters). |
| `orbit docs show <path>` | Render one doc with parsed frontmatter and body. |
| `orbit search --kind doc <query>` | Ranked matches against `summary`, `tags`, and `type`. Add `--hybrid` to blend lexical doc scoring with doc embeddings from `orbit docs index`. |
| `orbit docs add <path>` | Append `<path>` to `[docs].roots`. Idempotent. Refuses `.orbit/` paths and non-existent paths. |
| `orbit docs index` | Walk configured roots, embed doc fields into `.orbit/state/semantic.db`, and sweep stale doc rows. |
| `orbit docs migrate [--dry-run]` | One-shot frontmatter backfill for legacy `docs/design/<feature>/*.md` and `docs/design-patterns/*.md`. Idempotent. Never touches `.orbit/`. |

Each verb has an MCP twin (`orbit.docs.list`, etc.) registered in the safe MCP surface. CLI and MCP shapes are identical.

### 2.5 The `.orbit/` exclusion

The walker explicitly skips any path under `.orbit/`, even if a configured root accidentally points above it. ADRs at `.orbit/adrs/{accepted,proposed,superseded}/ADR-NNNN/` are owned by `orbit-adr` and have their own `orbit.adr.*` surface — they are *not* indexed by orbit-docs in v1. Whether orbit-docs eventually absorbs them is the [ORB-00169] design question. The locating principle behind this boundary: [ADR-0170].

### 2.6 Learning vs. doc

The boundary is now explicit:

- **Learning** = a load-bearing rule with a known failure mode. CRUD'd via `orbit.learning.*`. Supersedable. Scope-glob push-injected. Lives at `.orbit/learnings/`.
- **Doc** = explanatory context. PR-reviewed Markdown under `docs/`. No supersede flow. Pull-retrieved via `orbit.docs.*`. Authors link to load-bearing learnings via `related_artifacts: [L-NNNN]` when useful.

If you find yourself wanting to write "rule: do X because Y" in a doc, that's a learning. If you find yourself wanting to write a multi-paragraph explanation of *why* a rule exists, that's a doc that links to the learning.

---

## 3. At a Glance

| Concern | File / surface | Task |
|---------|----------------|------|
| Frontmatter parsing, tolerant fallback, walker | [crates/orbit-core/src/command/docs.rs](../../../crates/orbit-core/src/command/docs.rs) | [ORB-00163] |
| CLI verbs (`orbit docs list/show/search/add/reindex/migrate`) | [crates/orbit-cli/src/command/docs.rs](../../../crates/orbit-cli/src/command/docs.rs) | [ORB-00163] |
| MCP tool registry exposure | [crates/orbit-cli/src/command/mcp/mod.rs](../../../crates/orbit-cli/src/command/mcp/mod.rs) | [ORB-00163] |
| Tool host dispatch | [crates/orbit-core/src/runtime/orbit_tool_host/docs_tools.rs](../../../crates/orbit-core/src/runtime/orbit_tool_host/docs_tools.rs) | [ORB-00163] |
| Skill (agent-facing entry point) | [crates/orbit-core/assets/skills/orbit-docs/SKILL.md](../../../crates/orbit-core/assets/skills/orbit-docs/SKILL.md) | [ORB-00163] |
| Config root | `[docs].roots` in [.orbit/config.toml](../../../.orbit/config.toml) | [ORB-00163] |
| Backfill migrator | `orbit docs migrate` | [ORB-00163] |
| Internal hardening (real diff, robust YAML edit, batched gitignore) | [crates/orbit-core/src/command/docs.rs](../../../crates/orbit-core/src/command/docs.rs) | [ORB-00164] |
| Retire `orbit-design` skill | [crates/orbit-core/assets/skills/orbit-design/](../../../crates/orbit-core/assets/skills/orbit-design/) | [ORB-00165] |
| Inject into `task show --with-context` | [crates/orbit-cli/src/command/task/](../../../crates/orbit-cli/src/command/task/) | [ORB-00166] |
| Extend PreToolUse hook to surface docs | [crates/orbit-core/src/command/learning_hook.rs](../../../crates/orbit-core/src/command/learning_hook.rs) | [ORB-00167] |
| Semantic embeddings ranker (v2) | [crates/orbit-core/src/command/semantic.rs](../../../crates/orbit-core/src/command/semantic.rs) | [ORB-00168] |
| Fold `.orbit/adrs/` into corpus (v2 design) | [.orbit/adrs/](../../../.orbit/adrs/) | [ORB-00169] |

---

## Task References

- [ORB-00163] — Introduce `orbit docs` indexed knowledge base and `orbit-docs` skill (shipped)
- [ORB-00164] — Harden orbit-docs internals: real diff, robust YAML edit, gitignore caching
- [ORB-00165] — Retire `orbit-design` skill in favor of `orbit-docs`
- [ORB-00166] — Wire `orbit docs` retrieval into `task.show --with-context` and `task.start`
- [ORB-00167] — Extend PreToolUse hook to surface relevant docs alongside learnings
- [ORB-00168] — Add semantic embeddings index for orbit-docs corpus (v2)
- [ORB-00169] — Design: fold `.orbit/adrs/` into the orbit-docs corpus (v2)

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
