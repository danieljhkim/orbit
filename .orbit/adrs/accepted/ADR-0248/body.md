## Context

[ORB-10346] removed the learning-reminder PreToolUse registrations this repo's own `.claude/settings.json` and `.codex/config.toml` carried, per the pull-discovery direction (ADR-0108/ADR-0112 supersession, ADR-0242 amendment). It deliberately left the writer mechanism itself untouched — `orbit workspace init --hooks` and `orbit hook install` both still silently wrote those registrations back for any repo that invoked them, and the tree still carried two now-inert tracked shim files (`.claude/hooks/orbit-learning-reminder`, `.codex/hooks/orbit-learning-reminder`) left over from before the retirement. [ORB-10366] closes that gap.

Two call sites write the registration via `orbit_cmd::hook_install::install_for_workspace`:

1. `orbit workspace init --hooks` (`crates/orbit-cli/src/command/workspace/init.rs`) — an implicit side effect of the init flow, easy to invoke unintentionally from a script or muscle memory (`orbit init --hooks` from an older doc, a stale redeploy script, etc.).
2. `orbit hook install` (`crates/orbit-cli/src/command/hook/install.rs`) — a standalone, explicitly human-invoked command with no other purpose.

## Decision

Remove the `--hooks` flag from `orbit workspace init` entirely (clap now rejects it as an unknown argument rather than silently ignoring it) and delete the two tracked inert shim files. Keep `orbit hook install` / `orbit hook uninstall` as an explicit, opt-in, human-invoked escape hatch — do not remove the command.

The distinction that matters is *automatic* vs. *deliberate*. ADR-0108/ADR-0112's problem was learnings being pushed into agent context without anyone choosing that — the failure mode was "agents not knowing they should look," closed by pull discovery instead. An `--hooks` flag riding along on `workspace init` (a command run for unrelated reasons — bootstrapping a new checkout) reproduces exactly that: a side effect nobody asked for in the moment. `orbit hook install` has no such ambiguity — it is the only thing the command does, so running it is itself the deliberate choice, same category as a human explicitly opting back into the old delivery model for a specific reason (e.g. a non-Claude-Code agent runtime that has no other discovery path, or reproducing pre-retirement behavior for comparison). Removing it forecloses that choice for no safety gain, since `orbit hook uninstall` (needed regardless, to clean up pre-ORB-10366 registrations like this repo's own) is already the same shape of explicit command.

## Consequences

- No code path reachable from `orbit init` or `orbit workspace init` writes a learning-reminder registration; a fresh `workspace init` against a temp workspace root leaves `.claude/settings.json` / `.codex/config.toml` untouched (asserted in `crates/orbit-cli/tests/hook_install.rs`).
- `orbit workspace init --hooks` now fails with a clap "unexpected argument" error instead of silently succeeding — a stale script or doc still passing it breaks loudly at the call site instead of appearing to work.
- `orbit hook install` / `orbit hook uninstall` are unchanged; `orbit_cmd::hook_install::{install,uninstall}_for_workspace` and the underlying JSON/TOML merge helpers are untouched, so the only diff is at the two CLI call sites plus the flag itself.
- The independently-registered `scripts/orbit-file-lock` PreToolUse guard and the `orbit hook pretooluse` / `learning_hook::run_pretooluse` runtime path are unrelated to this change and keep working exactly as before.
- Cost: `orbit hook install` remains capable of re-registering the retired delivery mechanism if a human runs it deliberately. This is accepted as the intended behavior of an opt-in escape hatch, not a gap — the alternative (removing the command) would require touching `orbit-cli`'s audit-metadata match, `operation.rs`'s exhaustive command arm, and the `--help` template for a command with a legitimate use case and no automatic trigger.
- Cost: the two tracked inert shim files this repo carried are deleted rather than kept as reference examples; `orbit hook install` regenerates them byte-identically if ever re-run, so nothing is lost.