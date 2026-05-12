# CLAUDE.md

Project instructions for agents working on Orbit.

## Rules

- **Don't commit** until the Orbit task has been explicitly approved by the human.
- **Don't invent task IDs** — get them from `orbit.task.add`. Don't edit task files directly — use `orbit.task.update`.
- **Don't add cross-crate dependencies** without checking the architecture diagram below.
- **Use subagents** for large tasks to keep your context window clean.

## Build / Lint

`make build`, `make fmt`, `make ci` — all must pass before a task moves to `review`.

## Design Docs

Feature design docs live under `docs/design/<feature>/` and follow [`CONVENTIONS.md`](docs/design/CONVENTIONS.md) (folder layout, required sections, ADR format, glossary shape).

**Update in the same PR as the implementation change.** Flip affected ADR statuses (`Proposed → Accepted` with task ID), bump `Last updated`, add a new ADR for any non-obvious decision the change embodies. Stale docs are a review blocker.

**Feature Lead responsibility.** The lead is accountable for their feature's design-doc hygiene and create tasks to ensure the docs are maintained, as well as proactively identifying design-related concerns via filing friction tasks.

| Feature | Folder | Lead |
|---------|--------|------|
| Knowledge graph | `knowledge-graph/` | `claude` |
| Policy & Sandboxing | `policy-sandbox/` | `claude` |
| Project Learnings | `project-learnings/` | `claude` |
| Task | `task/` | `codex` |
| Task Lineage | `task-lineage/` | `claude` |
| Semantic Search | `semantic-search/` | `claude` |
| Task Sync | `task-sync/` | `claude` |
| Activity / Job | `activity-job/` | `codex` |
| Auditability | `auditability/` | `codex` |
| Groundhog | `groundhog/` | `codex` |
| User Interface | `user-interface/` | `gemini` |

## Crate Architecture

```
orbit-common → orbit-policy, orbit-exec, orbit-knowledge, orbit-embed → orbit-tools → orbit-agent → orbit-engine → orbit-core → orbit-cli
            ↘ orbit-store ──────────────────────────────────────────────────────────────────────────↗            ↗
            ↘ orbit-embed-companion (depends on orbit-embed; not linked into the default `orbit` binary)
            ↘ orbit-mcp ─────────────────────────────────────────────────────────────────────────────────────────↗
```

`orbit-core` also depends directly on `orbit-knowledge` for graph workflow wrappers in `command::graph`.

## Maintainer Conventions

- Order files for the next maintainer's first read: lead with the module's primary concept or entry point, then move into supporting detail.
- Data-heavy modules usually start with their core structs/enums; orchestration modules usually start with the main public function.

**Rust practices:**

- **Errors:** propagate via `OrbitError` at crate boundaries; reach for typed `thiserror` variants over ad-hoc strings. `?` over manual `match`. Don't `unwrap` / `expect` in non-test code unless the invariant is local and stated in the message.
- **Logging:** use `tracing` (never `println!` / `eprintln!` / `log`). The default subscriber handles redaction — don't reach around it. Prefer structured fields (`tracing::info!(run_id, ...)`) over string interpolation.
- **Visibility:** default to `pub(crate)`; reserve `pub` for items in the crate's documented public surface (see the architecture diagram). Re-export at the crate root only for types that are genuinely part of the API.
- **Async locking:** never hold a `Mutex` / `RwLock` guard across `.await`. Scope the lock to a block, or use `tokio::sync` primitives when state is genuinely cross-task. Bounded channels by default.
- **Tests:** in-file unit tests under `#[cfg(test)] mod tests`; integration tests under `tests/`. Match nearby scaffolding (e.g. sibling `*_tests.rs` files in `orbit-engine::activity_job::job_executor`). Don't introduce a new test harness when an existing one fits.
- **Comments & docs:** `///` doc-comment public items. Explain *why* in code comments, not *what* — well-named identifiers carry the *what*. Remove commented-out code instead of fencing it.

## Commits & Authorship

- Use the agent commit identity (e.g. `codex`, `claude`) as author/committer.
- Include the Orbit task ID in commit messages when applicable (e.g. `[ORB-00042]`). Task IDs are allocation-authority search keys (`git log --grep '[ORB-00042]'`); when a task has a linked `external_ref`, include that tag too (`[ORB-00042] [ENG-1234] ...`) — cross-engineer reviewers resolve the external tag, not the Orbit one.
- Use your model name (e.g. `claude-opus-4-7`, `gpt-5.5`, `gemini-3.1-pro`) when authoring tasks or docs. Cite relevant task IDs in any doc you write.

## Orbit Workflow

For any Orbit lifecycle work (creating tasks, executing, reviewing, raising PRs), invoke the relevant `orbit-*` skill. The `orbit` skill is the entry point and router. Task authoring quality standards live in `orbit-create-task`.

Scoreboards live at `.orbit/state/scoreboard/` (e.g. `duel_plan.json` — planning-duel run results).
