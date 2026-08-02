## Context

[ADR-0306] resolves the output sink once per invocation and requires every renderer to read it rather than re-derive terminal state. [ADR-0308] adds that color emission must be decided in exactly one place. Both presuppose that a renderer can *reach* the sink.

It cannot, by parameter. Rendering in `orbit-cli` happens inside `Execute::execute(self, &OrbitRuntime) -> Result<(), OrbitError>`, which takes no renderer argument, across 154 `impl Execute` blocks and 98 command modules that call `println!` or `Table::print()` directly. `output/table.rs::Table::print` and `output/color.rs` are free functions those bodies call; neither has a call-site-provided sink to consult.

[ADR-0306]'s migration step 3 changes that signature so commands return payloads, and is explicitly the expensive step. [ORB-10570]'s scope — gate color and width at the sink — is independent of it and was sequenced before it precisely so that `NO_COLOR=1` stops emitting escape sequences without waiting on a 154-impl refactor.

So the sink has to be reachable from a free function called by a body that was not given one.

## Decision

`main` resolves the sink and publishes it into a `OnceLock` (`output::sink::install`); renderers read `output::sink::active()`.

- Before `install` runs, `active()` answers with a **piped** sink: not a terminal, width 0, no color, plain mode. The default is chosen so the failure mode of forgetting to install is unstyled untruncated output, never escape sequences written into a file.
- A duplicate `install` is logged at `debug` and ignored, not a panic. The sink is read-mostly configuration; aborting a user's command over it, or changing rendering halfway through one, are both worse than the second call being inert.
- `main` also calls `sink.apply_color_policy()`, which overrides the `colored` crate's own process-global env detection. `comfy_table` has no equivalent global, so `output::table` passes the same answer per render via `enforce_styling` / `force_no_tty`.
- Tests never call `OutputSink::from_process`. They build a sink with `OutputSink::resolve` and pass its answers to `Table::render(width, styled)` directly. The one test that flips the `colored` global serializes on a mutex and restores detection on drop.

Rejected alternative: **thread the sink through `Execute::execute`.** This is the correct end state and is what [ADR-0306] step 3 does. Rejected *for now* because it is the same 154-impl signature change that step 3 already owns; doing it here would either duplicate that churn or merge two independently reviewable changes, and would delay the `NO_COLOR` fix behind it. When step 3 lands, `active()` becomes removable in favor of the threaded value, and this ADR should be superseded rather than extended.

Rejected alternative: **have each renderer call `OutputSink::from_process()` itself.** Cheap and needs no global, but it re-derives terminal state per render — exactly the drift [ADR-0306] exists to remove — and would re-issue a `TIOCGWINSZ` ioctl per table. It also defeats `scripts/check-terminal-state-guard.sh`, whose whole premise is that one file queries the environment.

Rejected alternative: **a thread-local rather than a `OnceLock`.** Correct for concurrent in-process consumers, but `orbit-cli` renders from one thread and a thread-local would silently answer "piped" on any worker thread that rendered, which is a harder bug to see than the limitation below.

## Consequences

- `Table::print`, `output::color`, and `command/log/tail.rs` all read one answer, so the two styling backends can no longer disagree about `NO_COLOR`. That is the property [ORB-10570] exists to establish, and it is testable without a running `main`.
- The guard script's allowlist shrinks to the sink and its tests; `command/log/tail.rs` is no longer a grandfathered exception.
- Cost: a renderer's behavior depends on whether `main` ran. A unit test gets the piped default, which is the safe answer but not the interactive one, so a test that means to exercise the terminal path must build a sink explicitly and pass it — `Table::render(width, styled)` exists for exactly that, and `Table::print()` is the only function that reads the global.
- Cost: two sinks cannot be active concurrently in one process. An embedded or in-process invocation that wanted to render for a different destination would have to wait for the threaded sink of [ADR-0306] step 3. No such consumer exists today.
- Cost: `apply_color_policy` mutates the `colored` crate's process-global override, so any test that asserts on styled output must serialize against it. One mutex in `output/tests/gating.rs` carries that today; a second such test suite would have to share it rather than add its own.