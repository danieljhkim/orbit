# Orbit — agent guide

Project instructions for agents working on Orbit (loaded as both `AGENTS.md` and `CLAUDE.md`).

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


## Architecture

Crate layering, per-crate responsibilities, and scoping rules live in [`ARCHITECTURE.md`](ARCHITECTURE.md). Read it before adding a new crate, a new dependency edge, or a new persisted artifact.

Reusable codebase-specific patterns (Command, RAII guard, newtype, crate-boundary error translation) live in [`docs/design-patterns/`](docs/design-patterns/). When you reach for one of those shapes, copy from the documented reference instead of inventing a new one.

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
