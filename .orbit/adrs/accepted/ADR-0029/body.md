## Context
V2 JSONL and blob traces are runtime artifacts, but their old first-level `.orbit/audit/` path blurred command audit, workspace state, and authored docs.

## Decision
Store activity/job envelopes, loop events, and blobs under `.orbit/state/audit/`; keep command audit rows in the configured SQLite database.

## Consequences
- Runtime traces live with other workspace-local run state.
- Cost: old `.orbit/audit/` artifacts may need manual fallback or migration for historical reconstruction.
