## Context
`task.delete` could either remove the task directory from the orphan branch or leave it and add a marker indicating deletion. Hard removal is intuitive but creates a footgun: engineer A deletes [T20260504-7]; meanwhile engineer B (offline) made an edit to it; engineer B comes online and pushes; the task reappears as a "new" task with the same ID at the time of B's edit. Tombstones prevent this resurrection by recording the deletion as an explicit operation that subsequent operations honor.

## Decision
`task.delete` writes a `_tombstones/<task-id>.yaml` entry (with deletion timestamp, deleting agent, and the deleted task's last-known status) and removes the regular task path. Reads ignore tombstoned IDs. Operations against a tombstoned task fail with a clear error referencing the tombstone.

## Consequences
- Deleted tasks stay visibly deleted across syncs.
- Audit trail of deletions is preserved on the branch — a deleted task's lifecycle is still inspectable via `git log refs/heads/orbit/tasks -- proposed/<task-id>/`.
- Cost: tombstones accumulate without bound. A future ADR adds a tombstone-pruning policy (e.g., tombstones older than 6 months are hard-removed) once the operational signal exists. Until then, tombstones are forever.

---

## Task References

- [T20260505-12] — Design git-orphan-branch task sync (v2 feature). The task that produced this folder.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
