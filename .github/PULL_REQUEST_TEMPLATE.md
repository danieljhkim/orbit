<!--
Thanks for sending a pull request. Fill in the sections below so reviewers can
verify scope, run the same validation locally, and confirm docs were updated when needed.
-->

## Linked Orbit task(s)

<!--
Required. List the Orbit task ID(s) this PR implements, one per line.
Example: [ORB-NNNNN] — short title
If the task has a linked external tracker ref (Jira / Linear / GitHub issue),
include that tag too: [ORB-NNNNN] [ENG-1234] — short title
-->

- [ORB-NNNNN]

## Summary

<!--
What changed and why, in 1–3 sentences. Focus on intent, not a diff dump.
-->

## Test plan

<!--
Commands you actually ran, with results. At minimum:
-->

- [ ] `make ci` passes locally (runs fmt-check, build, clippy `-D warnings`, tests)
- [ ] Targeted tests for the affected crate(s) pass
- [ ] Manual verification steps (if applicable):

## Design docs

<!--
If this PR touches behavior described in `docs/design/*`, update the affected
docs in the SAME PR: flip ADR statuses, bump `**Last updated:**`, add new ADRs
for non-obvious decisions.
-->

- [ ] N/A — this PR does not touch any code referenced by `docs/design/*`

## Notes for reviewers

<!--
Optional. Call out risky areas, rollout plan, follow-ups deferred to other
Orbit tasks, or anything reviewers should look at first.
-->
