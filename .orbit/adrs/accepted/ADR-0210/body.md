## Context

The learning subsystem shipped two auxiliary surfaces beside the core add/update/supersede/evidence primitives:

- **Votes** (`orbit.learning.upvote`, `votes.jsonl` sidecar, `learning_vote_summary`, decay-weighted vote score in search ranking): a task-anchored upvote used as a secondary rank key.
- **Comments** (`orbit.learning.comment.{add,list,delete}`, `comments.jsonl` sidecar, `LearningReminder.comments`, a free-text redaction policy in `artifact_redaction.rs`): free-text footnotes anchored to a learning, injected under the learning summary in reminder blocks.

Across orbit's own `.orbit/learnings/` corpus (50 records), exactly one record had any comments (L-0005, one comment) and zero had any votes. The team had already half-retired both in ORB-00289/00348: `orbit.learning.upvote`, `orbit.learning.comment.list`, and `orbit.learning.comment.delete` were flipped to `register_inactive` (operator-only, off the agent-facing MCP surface). This ADR records the decision to finish the direction and remove both surfaces entirely.

The alternative that was on the table was **keep both, narrower**: keep `comment.add` as a lightweight annotation channel (accepted, but with clearer wording about when to prefer `update`/`supersede`), and keep `upvote` as a signal channel for "this learning is still useful" recency without ranking impact. That alternative was rejected below.

## Decision

Remove the vote and comment surfaces from the learning subsystem entirely. Concretely:

- Delete the tool definitions `orbit.learning.{upvote,comment.add,comment.list,comment.delete}` and their `OrbitBuiltinAction` variants; drop them from the tool registry, MCP host `LEARNING_TOOL_NAMES`, and both `INACTIVE_TOOL_NAMES` canaries.
- Delete the CLI subcommands `orbit learning upvote` and `orbit learning comment`.
- Delete the store surface: `LearningStoreBackend::{upvote_learning,learning_vote_summary,add_learning_comment,list_learning_comments,delete_learning_comment}` and their file-backend impls; `votes.jsonl`/`comments.jsonl` layout paths; `LEARNING_COMMENTS_FILE_NAME`; `next_learning_comment_id`, `validate_learning_comment_id`, and comment JSONL record helpers.
- Drop the scoreboard `learning_votes_received` column (from both the Rust struct and the dashboard `scoreboard.js` renderer/aggregator).
- Drop `LearningComment`, `LearningCommentEvent`, `LearningCommentTombstone`, `LearningVoteRow`, `LearningVoteSummary`, `DEFAULT_LEARNING_COMMENT_RENDER_CAP`, `read_comment_render_cap_env`, `decayed_vote_score`, and `NotFoundKind::LearningComment` from `orbit-common`.
- Drop the `comments: Vec<LearningComment>` field from `LearningReminder`; the reminder block now renders `- [id] summary` per record with no nested footnotes.
- Drop the `orbit.learning.comment.add` policy entry from `artifact_redaction.rs` and its `ArtifactTarget { artifact_type: "learning_comment", ... }` mapping.
- Migrate the single existing comment (L-0005/C20260519-1, about `include_str!` entries in `crates/orbit-core/src/command/skill.rs`) into the L-0005 learning body via `orbit.learning.update` before the surface is deleted, so the datum is preserved.

## Consequences

- **Corrections and provenance are funneled through the primary surfaces.** Curators correct current wording with `orbit.learning.update`, mark material changes with `orbit.learning.supersede`, and cite provenance with the `evidence` array (`{kind, ref}`). That was already the documented pattern; comments were a weak middle-ground primitive that muddied it.
- **Search ranking is now `priority` desc → `updated_at` desc → `id` asc.** The decay-weighted vote score dropped out of the primary sort key. Since no learning had votes, this changes no observed rankings today; if a reason to re-rank by recency of validation returns, `updated_at` (bumped on every `update`) is the natural signal — no new dedicated store is needed.
- **Attack surface shrinks.** Free-text comments required the `LearningCommentAdd` entry in the artifact-redaction policy (`body` scrubbed for env-injected credentials and home-dir paths). That policy entry, its `learning_comment` artifact-target mapping, and the comment-only branch of the audit-emit path are gone.
- **Store layout is simpler.** Each learning is now `<L-id>/learning.yaml` and nothing else in the common case. `sync`/reindex no longer needs to validate comment JSONL files. Rollback of a partial create no longer needs to remove sidecar files.
- **`LearningReminder` is smaller and cheaper to hydrate.** No per-reminder comment scan; the reminder block loses its footnote lines. Consumers of the reminder JSON (v2 host, MCP sidecar, CLI hook renderers) all lose the optional `comments` field; a client that hand-constructed a reminder with `comments: []` no longer compiles.
- **A follow-up avenue exists if voting-style feedback is ever needed.** `friction.add` covers "this learning is wrong / has bit me" as an incident channel that already routes to the human triage surface; `supersede` (with the new record's `evidence` carrying the incident's task ID) covers the material-change path. Nothing about this removal precludes reintroducing a scoped feedback primitive later, but it should be driven by real usage, not carried speculatively.
- **Cost: this is a breaking change to the tool and store surfaces.** External callers of `orbit.learning.upvote`/`comment.*`, of the store trait methods, or of the removed types on `orbit_common`/`orbit_core` public API must be updated in the same release; the length canary in `EXPECTED_INACTIVE_TOOL_NAMES` moves from 26 to 22. The change is documented in CHANGELOG.md's Breaking Changes section under ORB-10046.

### Rejected alternative: keep `comment.add` as an annotation channel; keep `upvote` for recency

Rejected because (a) usage is zero-to-one after months of availability, so we would be defending a feature with no evidence of demand; (b) `update` on an existing learning already carries the author's `model` and bumps `updated_at`, subsuming any recency signal a vote would carry; (c) `evidence` on `add`/`update` carries provenance in structured form that comments cannot; (d) keeping comments preserves the free-text redaction burden that drove ~50 LOC in `artifact_redaction.rs` plus its tests; (e) the store cost — two extra JSONL sidecars per learning, ID allocators for `C<YYYYMMDD>-N` comment IDs, tombstone events, and reindex validators — is not justified by one comment across the corpus. If a lightweight annotation channel is genuinely useful later, it can be reintroduced with actual usage data behind it.