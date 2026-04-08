# Contributing to Orbit

Thanks for contributing to Orbit.

## Principles

- Prefer simple, coherent designs over preserving accidental complexity.
- Fix root causes when practical, not just symptoms.
- Keep command, engine, executor, store, and type boundaries clean.
- Treat agent and human experience as product concerns, not just implementation details.

## Setup

```bash
cargo test --workspace
```

Use targeted tests while iterating, then run the full workspace suite before landing a change.

## Repository Shape

Rust workspace crates live under `orbit/` (for example `orbit/orbit-cli`).

- `orbit-cli`: CLI entrypoint
- `orbit-core`: composition root, command handling, runtime wiring
- `orbit-engine`: job and activity execution engine
- `orbit-tools`, `orbit-agent`, `orbit-store`, `orbit-types`, `orbit-policy`, `orbit-exec`: supporting runtime layers

## Change Expectations

- Keep changes scoped and intentional.
- Add or update tests when behavior changes.
- Prefer removing legacy paths over carrying compatibility code when the product is still pre-adoption.
- If you discover friction or recurring issues, fix them in scope or create a concrete follow-up task.

## Orbit State

Orbit keeps operational state under `.orbit/`. Review those changes carefully before committing.

- Do not accidentally commit noisy runtime artifacts.
- Treat tracked asset changes as product changes.
- Treat mutable run/task state as operational data unless the change is intentional.

## Commits

- Use clear commit messages.
- Agent-authored commits should use the agent commit identity for that commit.
- Do not leave the repository configured with the agent identity afterward.
