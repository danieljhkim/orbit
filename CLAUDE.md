# Orbit — agent guide

Project instructions for agents working on Orbit (loaded as both `AGENTS.md` and `CLAUDE.md`).

## Rules

- **Don't commit** until the Orbit task has been explicitly approved by the human.
- **Don't invent task IDs** — get them from `orbit.task.add`. Don't edit task files directly — use `orbit.task.update`.
- **Don't add cross-crate dependencies** without checking and updating [`ARCHITECTURE.md`](ARCHITECTURE.md). If a new edge is genuinely needed, make its ownership and direction explicit in the same change.
- Historical ADRs are being retired and are not an authority. Do not search for,
  cite, or use ADRs to justify a decision. Judge from the current code, runtime
  behavior, tests, documented constraints, and the requirements at hand.

## Branching

- **`main`** is the release / production branch — only release merges and hotfixes land here. Default base for external install URLs, npm/Homebrew consumers, and the GitHub default-branch view.
- **`agent-main`** is the dev integration branch — every task PR targets `agent-main`.
- **Promotion**: each release tags on `agent-main`, then merges `agent-main → main` via a merge commit. See [`RELEASING.md`](RELEASING.md) §10b.
- **Hotfixes** branch from `main`, merge to `main`, tag a patch release on `main`, then back-merge `main → agent-main` in the same session. See [`RELEASING.md`](RELEASING.md) §Hotfix flow.

## Build / Lint

`make ci-fast` (fmt-check + guardrail scripts; no compile) and `make ci-lint` (the same workspace-wide, all-target clippy pass as CI, with warnings denied) must both pass before a task moves to `review`. Each task therefore pays for one workspace clippy compile; cold runs can take several minutes, while warm runs reuse Cargo's incremental cache. The full `make ci` is the canonical merge gate via [`.github/workflows/ci.yml`](.github/workflows/ci.yml) on every PR — don't run it per task locally.


## Architecture

Crate layering, per-crate responsibilities, and scoping rules live in [`ARCHITECTURE.md`](ARCHITECTURE.md). Read it before adding a new crate, a new dependency edge, or a new persisted artifact.

Reusable codebase-specific patterns (Command, RAII guard, newtype, crate-boundary error translation) live in [`docs/design-patterns/`](docs/design-patterns/). When you reach for one of those shapes, copy from the documented reference instead of inventing a new one.

## Simplicity and ownership

- Optimize first for clarity and the fewest moving parts. Do not preserve a
  wrapper, compatibility layer, abstraction, or configuration path merely
  because it already exists. Keep compatibility only when an external contract
  or persisted format requires it, and state that constraint next to the code.
- Put each rule at its authoritative boundary. Transport and UI layers should
  collect inputs and adapt protocols; domain validation, authorization, and
  persistence invariants belong in the server/domain layer that can enforce
  them. Do not duplicate a server rule as client-side orchestration.
- Every crate and module must have one explainable job. If a dependency reads
  backwards, a file contains unrelated domains, or two crates contain the same
  helper, move the behavior to the lowest appropriate owner instead of adding
  another facade or forwarding chain.
- Treat roughly 800 lines in one source or test file as a design warning, not a
  target. Likewise, several related flat files (`task_add.rs`, `task_list.rs`,
  and so on) are a signal to create a domain module with sibling tests. Split by
  responsibility before adding more code.
- Treat a long function with several phases, policies, or failure modes as the
  same warning at function scale. Name the phases, extract cohesive helpers,
  and keep the top-level flow readable without jumping through empty wrappers.
- Prefer one canonical execution path and test it at that boundary. Thin entry
  points may attach context, but they must not grow parallel dispatch,
  validation, persistence, or audit implementations.
- Do not build speculative v2 machinery into a v1 change. Add the smallest
  complete seam the current behavior needs; introduce policy frameworks or
  generalized traits only when a concrete second use makes the boundary real.
- Delete dead application code and stale current documentation together.
  Preserve shipped migrations and durable data unless the change includes an
  explicit, tested compatibility plan.

## Design Docs

- **Layout.** Feature design docs live under `docs/design/<feature>/`. Keep current explanatory docs aligned with the implementation when they remain useful.
- **Same-PR updates.** Change affected current docs in the same PR as the code. Stale descriptions of live behavior are a review blocker.

## CHANGELOG entries

Don't modify `CHANGELOG.md` during task execution — it is compiled at release time from merged work, not accumulated per-PR. The task ID is the record of what changed; cite it in your commit message and let the release drafter pull from `git log` and Orbit task history. `scripts/check-changelog-style.sh` still lints any entries that do exist (harmless under this convention, and useful at release time). Full rule: [`RELEASING.md`](RELEASING.md) step 2.

## Rust Practices

- Concrete internal task, learning, and friction IDs are allowed in ordinary source comments, but never expose them in user-facing errors, CLI help/output, generated files, or advertised MCP/tool text; note that Clap renders `///` comments on commands and arguments as public help.

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

For any Orbit lifecycle work (creating tasks, executing, reviewing, raising PRs), invoke the `orbit` skill. Its `SKILL.md` is a router: load the reference that matches the job — `references/task-authoring.md` for authoring quality standards, `references/task-execution.md` for pickup through handoff, and so on.
