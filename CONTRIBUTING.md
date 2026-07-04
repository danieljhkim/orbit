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

Rust workspace crates live under `crates/` (for example `crates/orbit-cli`).

- `orbit-cli`: CLI entrypoint
- `orbit-core`: composition root, command handling, runtime wiring
- `orbit-engine`: job and activity execution engine
- `orbit-tools`, `orbit-agent`, `orbit-store`, `orbit-types`, `orbit-policy`, `orbit-exec`: supporting runtime layers

## Change Expectations

- Keep changes scoped and intentional.
- Add or update tests when behavior changes.
- Prefer removing legacy paths over carrying compatibility code when the product is still pre-adoption.
- If you discover friction or recurring issues, fix them in scope or create a concrete follow-up task.

## Supply-chain (cargo-deny)

Dependencies are gated by [`cargo-deny`](https://embarkstudios.github.io/cargo-deny/)
on every PR (via `scripts/ci-guardrails.sh`) and locally with `make audit`. The
policy lives in [`deny.toml`](deny.toml): it denies crates with an open RUSTSEC
advisory or a yanked version, and restricts licenses to a reviewed allow-list.

Run it before landing a dependency change:

```bash
cargo install cargo-deny --locked   # one-time
make audit                          # == cargo deny check
```

**Adding a license.** If a new dependency introduces a license not in the
`[licenses].allow` list, `cargo deny check` fails. Add the SPDX identifier to
the list in `deny.toml` **only** if it is a permissive/public-domain-equivalent
license, with a one-line comment naming the crate(s) and (for weak-copyleft
licenses such as MPL-2.0) a short justification. Copyleft licenses that would
impose obligations on Orbit's own sources must not be added — replace the
dependency instead.

**Advisory exceptions.** Only when there is no safe upgrade available may an
advisory be time-boxed in `[advisories].ignore`. Each entry must be an object
carrying:

- `id` — the `RUSTSEC-YYYY-NNNN` identifier, and
- `reason` — why it is safe in Orbit's usage (why the vulnerable path is
  unreachable or the impact is bounded) **and** a `Re-review YYYY-MM-DD` date
  (default: ~6 months out).

Re-review ignored advisories on or before their date and drop the entry once an
upstream fix lands. Never ignore an advisory that has an available patched
release — bump the dependency instead.

## Orbit State

Orbit keeps operational state under `.orbit/`. Review those changes carefully before committing.

- Do not accidentally commit noisy runtime artifacts.
- Treat tracked asset changes as product changes.
- Treat mutable run/task state as operational data unless the change is intentional.

## Commits

- Use clear commit messages.
- Agent-authored commits should use the agent commit identity for that commit.
- Do not leave the repository configured with the agent identity afterward.
