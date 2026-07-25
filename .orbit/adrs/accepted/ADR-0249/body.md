## Context
Pipeline-created commits exposed only a generic or family author even though the job run already persisted the exact resolved crew model used as `AGENT_MODEL` for provider subprocess commit trailers. The alternatives were to derive attribution again from `task.implemented_by` or crew aliases, or to let the author and trailer read different process state; both permit the ambient author to disagree with durable model telemetry.

## Decision
For workflow-owned commits created by Orbit, read the persisted job run `crew_model` once and use that same opaque string both to construct the author name `orbit (<model>)` and to set the spawned git process `AGENT_MODEL` consumed by `prepare-commit-msg`. Use the reserved non-routable email `agent@orbit.invalid`. Do not resolve aliases, validate model strings, or add a model registry. When the run model is absent or empty, omit `AGENT_MODEL` and use the generic `orbit <orbit@orbit.local>` author. Keep the committer as the process-scoped generic Orbit identity rather than ambient host config, so workflow commits remain deterministic and do not require or mutate user configuration. Existing commits are adopted without amendment.

## Consequences
- `git log --format=%an` distinguishes pipeline commits produced by different resolved models without inspecting messages.
- The `Agent-Model` trailer and model-bearing author cannot diverge inside a pipeline-created commit because both receive the same persisted value.
- Existing `Agent-Run`, `Agent-Task`, and co-author trailers remain additive and unchanged.
- ORB-10365 retains a host committer because its already-created commit was adopted forward-only, while ORB-10348 was created by pipeline automation with a scoped Orbit committer.
- Cost: an intentionally bare model value in `[crews.*].model` remains bare in the author, because Orbit treats configured model strings as opaque and does not ship a release-coupled alias table.