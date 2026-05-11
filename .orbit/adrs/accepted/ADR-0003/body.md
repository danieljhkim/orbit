## Context
Activity/job execution produces operator evidence at several layers: audit envelopes, job-run records, metrics, live traces, retained blobs, run-inspection commands, PR handoff summaries, and cancellation state. The separate ADRs all instantiate the same rule: runtime output is durable workflow state, not process stdout or live assets pretending to be history.

## Decision
Keep a v2 audit envelope layered over lower-level loop audit, persist direct and pipeline job runs as durable `JobRun` bundles, store file-backed traces under workspace state, read run inspection through runtime accessors, and place public run browsing under `orbit run`. CLI subprocess output may stream through tracing, but retained blobs remain archival; redaction belongs to the tracing subscriber; metrics, execution summaries, and cancellation are persisted as first-class run/task state.

Folded instances:

| ADR | Instance folded into this rollup |
|-----|----------------------------------|
| ADR-010 | Historical workflow inspection reads stored data, not live seeded assets. |
| ADR-012 | Direct v2 job runs persist durable job-run bundles. |
| ADR-017 | V2 job metrics persist invocation traces beside audit. |
| ADR-018 | File-backed run traces live under workspace state. |
| ADR-019 | Run inspection reads v2 traces through runtime accessors. |
| ADR-020 | Run inspection belongs to `orbit run`. |
| ADR-021 | CLI subprocess output is both a live tracing stream and retained audit blob. |
| ADR-022 | CLI output redaction belongs to the tracing subscriber. |
| ADR-036 | Task PRs require durable execution summaries. |
| ADR-038 | Dashboard cancellation is a durable job-run transition. |

## Consequences
- Reviewers can traverse runs by job, step, activity, and raw loop detail without parsing agent process output as workflow handoff.
- Operator surfaces share durable state for history, metrics, logs, cancellation, and PR handoff.
- The file layout clearly separates command audit queries from run-trace reconstruction files.
- Costs retained from folded entries:
- Cost: audit review now spans two related storage layouts instead of one.
- Cost: some read-only inspection paths no longer shared the same asset-validation gate as active workflow execution paths.
- Cost: direct v2 execution now has persistence side effects and can record synthetic job-level steps that were not literal authored YAML steps.
- Cost: job execution now has another persistence side effect, and CLI metrics remain limited by the provider harness output format.
- Cost: existing local `.orbit/audit/` artifacts are legacy files; readers looking for historical runs may need to check both locations during any manual transition period.
- Cost: the runtime layer now owns a read-side view model for audit JSONL, so envelope schema changes must update both writer and accessor tests together.
- Cost: scripts and muscle memory that used the removed aliases must migrate to the `orbit run` forms.
- Cost: CLI output now has two observability paths; the tracing line text is UTF-8/lossy and newline-stripped while the retained blob bytes remain the archival source.
- Cost: tests that inspect tracing safety must capture formatted subscriber output, not raw `Event` fields.
- Cost: manual or custom-body shipment paths must still persist task summaries before opening the PR, even when the caller already prepared a complete body.
- Cost: direct in-process job runs still cannot safely self-signal; dashboard cancellation is primarily the durable pipeline-worker/operator path.
