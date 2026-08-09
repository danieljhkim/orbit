## Context

`orbit adr` exposed only read/repair verbs (`list`, `show`, `restore`, `reconcile`). Authoring an ADR and moving one through its lifecycle existed only on the tool surface (`orbit tool run orbit.adr.add` / `orbit.adr.update`) and over MCP.

The gap bit hardest exactly where those surfaces are unavailable. An ADR authored inside a job worktree is federated relative to the hub, so a bridge/MCP write against it is refused with `artifact_not_local` (409) and can only succeed from the owning worktree. That leaves the operator on-box, inside that worktree, at a shell — and `orbit adr update <id> --status accepted` did not exist. Encountered 2026-08-08 accepting ADR-0328 in `orbit-jrun-20260808-2029-5`.

The open question ORB-10668 raised was whether `orbit adr reconcile` was already the intended answer, making this a discoverability defect rather than a missing verb.

## Decision

It is a missing verb, and `reconcile` addresses a different case.

1. Add `orbit adr add`, `orbit adr update`, and `orbit adr supersede`. Each is a thin `runtime.run_tool` delegation to the matching `orbit.adr.*` tool. The tool surface remains the single implementation of ADR semantics: ID allocation, the `proposed -> accepted` related-task rule, the refusal of direct `superseded` writes, the managed-run executor restriction, and the `artifact_not_local` federation guard all stay there. The CLI shapes argv into tool input and renders the response; it re-derives no rule.

2. `reconcile` is **not** the answer for the reported case. In the owning worktree the ADR resolves as `Local`, so `orbit adr update` succeeds directly and reconciling would be a no-op detour that also moves the bundle out of the checkout that owns it. `reconcile` remains the answer for the other direction — mutating an ADR *from* a checkout that does not own it, where the bundle must be brought in first.

3. The discoverability half is still real, so it is fixed as help text rather than as behavior: `orbit adr update --help` states the lifecycle transitions, and names `artifact_not_local`, the `artifact_origin` worktree, and the `reconcile` escape hatch — so the federated path is reachable from the CLI's own help instead of from a 409.

4. `command/adr.rs` becomes `command/adr/` (the documented parent-command directory shape in `crates/orbit-cli/CLAUDE.md`), one file per subcommand plus `support.rs`. At seven verbs the single file was already the largest under `command/`.

## Consequences

- The federated ADR path is completable with `orbit adr` alone from the owning worktree; no `orbit tool run`, no hand-edited `.orbit/adrs/`.
- Locality enforcement is untouched: the CLI never resolves artifacts itself, so a non-local target still fails closed with `artifact_not_local` and the full `artifact_origin` payload. A regression test in `crates/orbit-cli/tests/worktree_resolution.rs` pins both halves.
- Two surfaces now reach the same tools (CLI and MCP), so an `orbit.adr.*` schema change must consider both. That is already true of `list` and `restore`.
- `--status` is passed to the tool as an unparsed string rather than a clap `value_enum`, matching the existing `adr list --status` filter. Status vocabulary stays defined in one place; the cost is that an invalid value is reported by the tool rather than by clap.
- Cost: the CLI's ADR surface grows from four verbs to seven, and the directory split moves ~320 lines, so `git log --follow` on the old `command/adr.rs` path needs rename detection.

## Alternatives rejected

- **Treat it as pure discoverability and only document `reconcile`.** Rejected: it prescribes a bundle move for a case where the ADR is already local, and still leaves no CLI verb for the mutation itself.
- **Reimplement the lifecycle rules in the CLI for better clap ergonomics.** Rejected: it duplicates the `proposed -> accepted` and supersession rules across two surfaces that would then drift.
- **Relax the federation guard so the hub can write a non-local ADR.** Rejected outright: the guard is the reason a federated bundle stays committable from exactly one checkout.