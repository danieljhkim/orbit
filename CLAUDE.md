# CLAUDE.md

Project instructions for agents working on Orbit.

## Rules

- **Don't commit** until the Orbit task has been explicitly approved by the human.
- **Don't invent task IDs** — get them from `orbit.task.add`. Don't edit task files directly — use `orbit.task.update`.
- **Don't add cross-crate dependencies** without checking [`ARCHITECTURE.md`](ARCHITECTURE.md). If a new edge is genuinely needed, file a task and an ADR before adding it.
- **Use subagents** for large tasks to keep your context window clean.

## Branching

- **`main`** is the release / production branch — only release merges and hotfixes land here. Default base for external install URLs, npm/Homebrew consumers, and the GitHub default-branch view.
- **`agent-main`** is the dev integration branch — every task PR targets `agent-main`.
- **Promotion**: each release tags on `agent-main`, then merges `agent-main → main` via a merge commit. See [`RELEASING.md`](RELEASING.md) §10b.
- **Hotfixes** branch from `main`, merge to `main`, tag a patch release on `main`, then back-merge `main → agent-main` in the same session. See [`RELEASING.md`](RELEASING.md) §Hotfix flow.

## Build / Lint

`make ci-fast` (fmt-check + guardrail scripts; no compile) must pass before a task moves to `review`. The full `make ci` is the canonical merge gate via [`.github/workflows/ci.yml`](.github/workflows/ci.yml) on every PR — don't run it per task locally.

Mechanical lint rules belong in `[workspace.lints]` in the root `Cargo.toml`, not in this file. If PR CI fails for reasons unrelated to the task, record evidence and surface to a reviewer rather than broadening scope.

## Architecture

Crate layering, per-crate responsibilities, and scoping rules live in [`ARCHITECTURE.md`](ARCHITECTURE.md). Read it before adding a new crate, a new dependency edge, or a new persisted artifact.

Reusable codebase-specific patterns (Command, RAII guard, newtype, crate-boundary error translation) live in [`docs/design-patterns/`](docs/design-patterns/). When you reach for one of those shapes, copy from the documented reference instead of inventing a new one.

## Design Docs

- **Layout (recommended).** Feature design docs live under `docs/design/<feature>/` following the recommended folder layout, required sections, ADR format, and glossary shape documented in [`docs/design/CONVENTIONS.md`](docs/design/CONVENTIONS.md). Use the `orbit-docs` skill / `orbit docs` surface to retrieve indexed human-authored docs. The 4-numbered-doc structure and `Last updated:` convention are recommendations for coherence across authors, not rules enforced by `orbit-design` (retired) or any tool.
- **Same-PR updates.** Change the doc in the same PR as the code: flip affected ADR statuses (`Proposed → Accepted` with task ID), bump `**Last updated:**`, add a new ADR for any non-obvious decision the change embodies. Stale docs are a review blocker.

## Rust Practices

Lint-enforced rules:

- **Panic surfaces:** `[workspace.lints.clippy].unwrap_used` and `[workspace.lints.clippy].expect_used` are `warn` and therefore fail under `make ci`'s `-D warnings`, except for scoped test/example/invariant allowlists with comments. Prefer `OrbitError` propagation at crate boundaries, and use `expect("<invariant>")` only when the invariant is local and documented. See [`docs/design-patterns/error_translation.md`](docs/design-patterns/error_translation.md) for the boundary-translator shape.
- **Logging vs user output:** `[workspace.lints.clippy].print_stdout` and `[workspace.lints.clippy].print_stderr` are `warn` and therefore fail under `make ci`'s `-D warnings`, except for genuine CLI/example user-facing output allowlists. Use `tracing` for diagnostics, prefer structured fields (`tracing::info!(run_id, ...)`) over string interpolation, and rely on the default subscriber for redaction.
- **Async locking:** `[workspace.lints.clippy].await_holding_lock` is `deny`; never hold a `std::sync::Mutex` / `RwLock` guard across `.await`. Scope the lock to a block, or use `tokio::sync` primitives when state is genuinely cross-task.

Conventions (not lint-enforced):

- **Errors:** reach for typed `thiserror` variants over ad-hoc strings when translating into `OrbitError`.
- **Visibility:** default to `pub(crate)`; reserve `pub` for items in the crate's documented public surface (see `ARCHITECTURE.md`). Re-export at the crate root only for types genuinely part of the API.
- **Channels:** bounded channels by default.
- **Tests:** unit tests live in a `tests/` directory under each module — `src/command/skill.rs` declares `#[cfg(test)] mod tests;` and the tests sit at `src/command/skill/tests/mod.rs` with one submodule per concern. Test through each module's exposed surface (`pub` / `pub(crate)`); don't reach into private internals — let untestability surface as design pressure for cleaner seams. Crate-root `tests/` stays for true integration tests (separate binary, public API only). See [`docs/design-patterns/test_layout.md`](docs/design-patterns/test_layout.md) for the canonical structure. Existing inline `#[cfg(test)] mod tests` blocks and sibling `*_tests.rs` files are being swept to this layout under [ORB-00219] and per-crate sibling tasks. Don't introduce a new test harness when an existing one fits.

Related lint work:

- **Public docs:** `[workspace.lints.rust].missing_docs` is owned by sibling task ORB-00004; keep that migration separate from this broader Rust Practices lint pass.

## Commits & Authorship

- Use the agent commit identity (e.g. `codex`, `claude`) as author/committer.
- Include the Orbit task ID in commit messages when applicable (e.g. `[ORB-00042]`). Task IDs are allocation-authority search keys (`git log --grep '[ORB-00042]'`); when a task has a linked `external_ref`, include that tag too (`[ORB-00042] [ENG-1234] ...`) — cross-engineer reviewers resolve the external tag, not the Orbit one.
- Use your agent family (`codex`, `claude`, `gemini`, `grok`) for the `model` field when authoring tasks or docs — not a full model string. Full model strings are accepted and auto-normalized, but the family is the canonical identity. Cite relevant task IDs in any doc you write.

## Orbit Workflow

For any Orbit lifecycle work (creating tasks, executing, reviewing, raising PRs), invoke the relevant `orbit-*` skill. The `orbit` skill is the entry point and router. Task authoring quality standards live in `orbit-create-task`.

Scoreboards live at `.orbit/state/scoreboard/` (e.g. `duel_plan.json` — planning-duel run results).
