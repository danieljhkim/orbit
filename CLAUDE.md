# Orbit — agent guide

Project instructions for agents working on Orbit (loaded as both `AGENTS.md` and `CLAUDE.md`).

## OpenAI Build Week delivery directive — temporary

Through **2026-07-21 5:00 PM PT**, deliver the complete accepted
[`host-registry`](docs/design/host-registry/) and [`mcp-bridge`](docs/design/mcp-bridge/) designs.
Be submission-ready Monday; Tuesday is for final verification and submission, not first integration.

- Give 110%: no hidden scope cuts, shortcuts, skipped checks, or demo-only behavior. Read both
  designs' overview, design, and decisions files; keep their shared contracts integrated.
- Use only `sol` (default for planning, review, cross-crate, medium/large/complex/ambiguous work)
  and `terra` (small, easy, low-risk mechanical work).
- Raise better ideas, doubts, risks, and operational friction immediately with evidence and a
  recommendation. Never invent unclear semantics or silently work around broken infrastructure.
- Keep docs/ADRs synchronized, test success and failure paths, pass `make ci-fast` before review,
  and retain GitHub CI as the merge gate. Preserve concrete Codex/GPT-5.6 evidence for submission.

Primary `/feedback` session ID: `019f6e0f-eb02-73e3-b8ea-dbc8217ba57e`. Remove this section after
submission.

## Resident agent

**Hohmann** (`agentbase/hohmann/memory`) is Sol's resident systems engineer for Orbit. The shared
front-door orchestrator owns cross-workspace routing, dispatch, independent review, merge, and task
closure; Hohmann holds this repository's deep implementation context and executes one scoped Orbit
mandate at a time.

A direct Hohmann run must use Bridge `agent_invoke` with `provider="codex"` and
`model="gpt-5.6-sol"`; its prompt reads the on-box Hohmann memory layer first and states "you are
Hohmann, Sol's Orbit systems engineer." Generic Codex/Claude runs and independent Opus/Fable
reviewers may work on Orbit, but they do not impersonate Hohmann. The full invocation and
cross-codebase handoff contract lives in Hohmann's `CLAUDE.md`.

## Rules

- **Don't commit** until the Orbit task has been explicitly approved by the human.
- **Don't invent task IDs** — get them from `orbit.task.add`. Don't edit task files directly — use `orbit.task.update`.
- **Don't add cross-crate dependencies** without checking [`ARCHITECTURE.md`](ARCHITECTURE.md). If a new edge is genuinely needed, file a task and an ADR before adding it.

## Branching

- **`main`** is the release / production branch — only release merges and hotfixes land here. Default base for external install URLs, npm/Homebrew consumers, and the GitHub default-branch view.
- **`agent-main`** is the dev integration branch — every task PR targets `agent-main`.
- **Promotion**: each release tags on `agent-main`, then merges `agent-main → main` via a merge commit. See [`RELEASING.md`](RELEASING.md) §10b.
- **Hotfixes** branch from `main`, merge to `main`, tag a patch release on `main`, then back-merge `main → agent-main` in the same session. See [`RELEASING.md`](RELEASING.md) §Hotfix flow.

## Build / Lint

`make ci-fast` (fmt-check + guardrail scripts; no compile) must pass before a task moves to `review`. The full `make ci` is the canonical merge gate via [`.github/workflows/ci.yml`](.github/workflows/ci.yml) on every PR — don't run it per task locally.

## Agent Read Exclusions

Team-wide `Read()` exclusions (build artifacts, runtime state) live in [`.claude/settings.json`](.claude/settings.json) under `permissions.deny`. If you work on the excluded content itself (e.g. benchmark harness output), override locally in `.claude/settings.local.json` with a matching `allow` rule — don't relax the committed list.

## Architecture

Crate layering, per-crate responsibilities, and scoping rules live in [`ARCHITECTURE.md`](ARCHITECTURE.md). Read it before adding a new crate, a new dependency edge, or a new persisted artifact.

Reusable codebase-specific patterns (Command, RAII guard, newtype, crate-boundary error translation) live in [`docs/design-patterns/`](docs/design-patterns/). When you reach for one of those shapes, copy from the documented reference instead of inventing a new one.

## Code Navigation

This repo has a semantic graph available via the `orbit` MCP server (no live LSP):

- **Definition / signature lookup** → `orbit_graph_search`, then `orbit_graph_show` for file:line, signature, and doc comment without a `Read`. Selectors take the form `symbol:<file>#<name>:<kind>`; use a method-on-impl selector or `source_regex` when a plain name is ambiguous, and `include_non_code` for doc/config matches.
- **Find references / callers (who uses X?)** → **`orbit_graph_refs`** with `include: "all"`, *not* `orbit_graph_callers`. The `callers` index misses cross-crate calls that go through `pub use` re-exports (e.g. a symbol defined in `orbit-common`, re-exported from `orbit-core`, called in another crate), so it routinely returns empty for real public functions. `orbit_graph_refs` surfaces the actual call sites plus re-export points.
- **Ground-truth fallback** → `rg --type rust 'symbol_name'`. Use when `refs` looks incomplete or you need to see exact textual context (macro call sites, doc references, etc.).
- **From a plain shell (no MCP)** → the same queries are bundled in the main `orbit` binary as `orbit graph <sub>` (`search`/`show`/`refs`/`callees`/`impact`/`deps`/`trace`/`overview`/`implementors`, plus `sync`). In-process the MCP tools are faster — reach for the CLI only when the graph tools aren't available.

## Design Docs

- **Layout.** Feature design docs live under `docs/design/<feature>/`. Folder layout, required sections, ADR format, and glossary shape are documented in [`docs/design/CONVENTIONS.md`](docs/design/CONVENTIONS.md). Use the `orbit-search` skill / `orbit docs` surface to retrieve indexed docs.
- **Same-PR updates.** Change the doc in the same PR as the code: flip affected ADR statuses (`Proposed → Accepted` with task ID), bump `**Last updated:**`, add a new ADR for any non-obvious decision the change embodies. Stale docs are a review blocker.

## CHANGELOG entries

Don't modify `CHANGELOG.md` during task execution — it is compiled at release time from merged work, not accumulated per-PR. The task ID is the record of what changed; cite it in your commit message and let the release drafter pull from `git log` and Orbit task history. `scripts/check-changelog-style.sh` still lints any entries that do exist (harmless under this convention, and useful at release time). Full rule: [`RELEASING.md`](RELEASING.md) step 2.

## Rust Practices

Lint-enforced rules (full set in `[workspace.lints]`; key implications below):

- **No `unwrap()` / `expect()` at crate boundaries.** Propagate `OrbitError`; use `expect("<invariant>")` only when the invariant is local and documented. See [`docs/design-patterns/error_translation.md`](docs/design-patterns/error_translation.md).
- **No `print!` / `eprint!`.** Use `tracing` with structured fields (`tracing::info!(run_id, ...)`), not string interpolation. Allowlisted only for genuine CLI/example user output.
- **No lock guards across `.await`.** Scope `std::sync::Mutex` / `RwLock` to a block, or use `tokio::sync` for cross-task state.

Conventions (not lint-enforced):

- **Errors:** reach for typed `thiserror` variants over ad-hoc strings when translating into `OrbitError`.
- **Visibility:** default to `pub(crate)`; reserve `pub` for items in the crate's documented public surface (see `ARCHITECTURE.md`). Re-export at the crate root only for types genuinely part of the API.
- **Channels:** bounded channels by default.
- **Tests:** unit tests live in a *sibling* `tests/` directory mirroring source filenames (`src/command/skill.rs` → `src/command/tests/skill.rs`). The sibling layout structurally enforces public-surface testing. Crate-root `tests/` is for integration tests only. See [`docs/design-patterns/test_layout.md`](docs/design-patterns/test_layout.md). Don't introduce a new test harness when an existing one fits.

## Commits & Authorship

- Use the agent commit identity (e.g. `codex`, `claude`) as author/committer.
- Include the Orbit task ID in commit messages when applicable (e.g. `[ORB-00042]`). Task IDs are allocation-authority search keys (`git log --grep '[ORB-00042]'`); when a task has a linked `external_ref`, include that tag too (`[ORB-00042] [ENG-1234] ...`) — cross-engineer reviewers resolve the external tag, not the Orbit one.
- Use your agent family (`codex`, `claude`, `gemini`, `grok`) for the `model` field when authoring tasks or docs — not a full model string. Full model strings are accepted and auto-normalized, but the family is the canonical identity. Cite relevant task IDs in any doc you write.

## Orbit Workflow

For any Orbit lifecycle work (creating tasks, executing, reviewing, raising PRs), invoke the relevant `orbit-*` skill. The `orbit` skill is the entry point and router. Task authoring quality standards live in `orbit-task`.

Planning-duel scoreboards, when a duel has run, appear under `.orbit/state/scoreboard/` (e.g. `duel_plan.json`) — workspace-local runtime state, gitignored, so the path won't exist until then.
