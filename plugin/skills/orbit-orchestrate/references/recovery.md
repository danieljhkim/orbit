# Routing findings back into work

Post-merge review comments, QA findings, CI failures, and operational
incidents are not a separate track — they are new input to the same
preparation loop in [loop.md](loop.md), starting at "search open and closed
history" so a repair is never filed twice.

## CI findings file, they do not repair

`ci_failure_sweep_pipeline` files GitHub Actions findings as `proposed` and
runs task-pilot against each new one, admitting only current,
warning-free repairs to `backlog` — it never implements a fix itself. A repair
task produced this way still needs the same promotion review as any other
task before it ships: read its applied context and pilot warnings, don't
assume the sweep's own admission is sufficient if something looks off.
→ [orchestration.md](../orbit/references/orchestration.md),
[workflows.md](../orbit/references/workflows.md)

A **CI finding surfacing after a task already merged** is not a reason to
patch the merged branch by hand. Let (or trigger) the sweep file the finding,
confirm it reaches a promotable state through the ordinary pilot step, then
dispatch the repair like any other backlog task. →
[walkthroughs.md](walkthroughs.md)

## Blocked runs: triage first, then decide

`task_triage_pipeline` diagnoses tasks blocked by a failed run and separates
environmental casualties (a transient lock, a provider timeout, a sandbox
denial — re-backlogged automatically) from real failures (left `blocked` with
a diagnosis attached):

```bash
orbit run triage                    # every blocked task attributable to a failed run
orbit run triage <task-id> ...      # narrow the scan
```

Run it on a schedule or before a large dispatch — an un-triaged failed run
just parks a task in `blocked` forever. For a run that hasn't been triaged
yet, or whose diagnosis needs deeper reading, use
[run-debugging.md](../orbit/references/run-debugging.md)'s full flow: run
bundle, audit trail, logs, failure classification, task and git state.

**Repeated identical failures are evidence for a repair task, not for another
retry.** If triage or manual debugging shows the same task failing the same
way more than once, stop retrying it as-is. File a repair task describing the
actual root cause (with the failing run's evidence attached), let it go
through pilot and promotion like any other task, and only then dispatch it —
retrying blindly burns runs without ever fixing what triage already
identified.

## Prefer an actionable task over a bare friction record

When a CI/QA finding or an operational incident points at something concretely
wrong in the repository or its pipelines, file (or promote) a task that fixes
it — that is what actually closes the loop. Reserve
[friction.md](../orbit/references/friction.md) for recording that the
*tooling or diagnostics themselves* were misleading or missing something
(a confusing error, an undocumented failure mode) — a report about the
experience of doing the work, not a substitute for the fix itself. Often
both: file the friction for what made diagnosis harder, and a task for the
actual repair, linked with `relations: [{"type":"resolves","target":"<friction-id>"}]`.

## Never build a shadow store

A missing MCP operation, a denied capability, or a host you cannot currently
reach is reported, not worked around. Do not fall back to direct file edits
under `.orbit/`, a second local store, or a different host's CLI to make a
finding "go somewhere" — see
[tool-surface.md](../orbit/references/tool-surface.md)'s "Missing operations"
section and [walkthroughs.md](walkthroughs.md) for the concrete shape of that
conversation.
