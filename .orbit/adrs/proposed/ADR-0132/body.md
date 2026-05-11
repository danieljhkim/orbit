## Context
Standard git text merge fails for task bundles in three concrete ways: status-transition divergence (git keeps both target paths), comment/history list appends (text-merge mangles YAML structure), and same-field edits (humans can't usefully resolve YAML quoting conflicts). Four mechanisms were evaluated against four scenarios in [2_design.md §3.1](./2_design.md): ADD-only sync, operation-aware replay, event sourcing, and no sync. ADD-only ships fast but leaves the partial-coverage mental model that updates don't sync — the operations users care about most. Event sourcing handles every scenario but requires building an event materializer and abandoning YAML-as-canonical. No sync defers the entire problem to v2 shared-host.

## Decision
v2 sync uses operation-aware replay. The on-branch artifact remains a YAML snapshot; the client treats every push as one of a fixed set of operations (`task.add`, `task.transition`, `task.comment.append`, `task.history.append`, `task.review.append`, `task.field.update`, `task.artifacts.upsert`, `task.delete`) and replays the operation against the new tip when push is rejected. Operations that are convergent by construction (`comment.append`, `history.append`, `review.append`) always retry automatically. Operations that depend on a baseline (`transition`, `field.update`) check the baseline and either retry or surface a structured conflict.

## Consequences
- The on-branch artifact stays human-readable and inspectable with `git log`.
- Most concurrent operations replay automatically; only genuine same-field-edit conflicts surface to the user.
- Existing `orbit-store` code keeps owning YAML serialization and layout; the sync coordinator is a new layer above, not a rewrite of the store.
- Cost: when a structured conflict surfaces, the user must run `orbit task sync resolve <task-id>` and choose. The UX must be excellent or the feature becomes friction; we're explicitly trading "magic auto-merge" for "explicit, structured surface."

---
