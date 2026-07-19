## Context
Upfront job-run injection needs a stable exact-match bridge between task tags and learning tags. Free-form writes drift into aliases, while a host-global registry copy would separate the policy from the code review that changes it. The existing config.yaml identity-stub convention was the alternative, with behavior knobs left in config.toml.

## Decision
The workspace commits the canonical learning/task tag vocabulary and upfront injection cap under .orbit/config.yaml learnings. Tagged learning and task writes normalize then reject unknown tags; changing the vocabulary is therefore PR-gated. Job-run injection reads the same file, matches only vocabulary-approved task tags, ranks by matched-tag count then priority then recency, and fails open with no injection when the file is missing.

## Consequences
- Task and learning authors receive an actionable rejection instead of silently creating a tag that never joins the two artifacts.
- Existing config.yaml files without a learnings section receive shipped defaults; an explicit empty vocabulary disables upfront selection.
- Checkoutless hub task writes remain available when tagless, while tagged MCP writes require a bound checkout so the committed vocabulary can be consulted.
- Cost: config.yaml is no longer an identity-only stub, so every strict parser and workspace initializer must preserve the learnings section and vocabulary changes require code review.
- Cost: exact-match curation requires retaining legacy aliases until records are migrated, which makes the initial vocabulary larger than the desired steady-state set.