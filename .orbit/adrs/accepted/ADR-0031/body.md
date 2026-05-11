## Context
V2 job execution emits audit JSONL, but metrics and scoreboards read the invocation store. Scraping audit logs would couple reporting to transcript format and retention.

## Decision
Persist `InvocationTrace` records beside audit as first-class metric records keyed by job run, activity, task ids, agent, model, usage, and tool-call summaries.

## Consequences
- `orbit metrics` and scoreboards can avoid parsing audit JSONL.
- Cost: metrics can diverge from transcript detail if a provider path reports incomplete usage.
