## Context

[ORB-10538] shipped `orbit.adr.restore`, the exact-id repair for an ADR whose allocation survived but whose body did not. Applying it to the 18 records in [ORB-10479] exposed two gaps between that contract and the population it was built for.

First, the tool was registered with `register_inactive` and a comment stating it stayed "available to `orbit tool run`". It did not: `orbit tool run` dispatches through `execute_tool_command_dispatch_*`, which gates on `ensure_tool_agent_facing`, and that rejects every inactive tool. With no CLI subcommand either, the tool had no reachable caller at all.

Second, `restore_allocated_adr` resolved the allocation through `adr_allocation`, whose SQL excludes `status = 'abandoned'` rows. But [ORB-10501]'s `abandon_orphaned` marks an allocation abandoned precisely when its pinned worktree is reaped — the dominant cause of the body loss in [F2026-07-163]. Four of the 18 (ADR-0157, ADR-0211, ADR-0225, ADR-0259) were in that state and were unrepairable by the tool built to repair them.

The alternatives for the second gap were to leave abandoned rows unrepairable and re-allocate fresh IDs for them (rejected by [ORB-10458]: `orbit.adr.show` has no ID-to-legacy fallback at citation sites, so every inline `[ADR-0157]` reference would stay broken), or to hand-edit `.orbit/`, which the repo agent guide forbids.

## Decision

Exact-id ADR restore is an operator surface reached through `orbit adr restore`, and it repairs abandoned allocations as well as live ones.

1. `orbit adr restore` is a CLI subcommand that calls `runtime.run_tool`, which bypasses `ensure_tool_agent_facing` while preserving every guard the tool enforces. This is the same bypass `orbit adr list` uses for the same reason ([ORB-00289]); registering a tool inactive is a statement about the *agent* surface, and any inactive tool that operators must still invoke needs a CLI subcommand to be reachable.
2. `restore_allocated_adr` resolves its allocation through `adr_allocation_for_restore`, which includes `abandoned` rows. This is sound because an abandoned row still owns its ID permanently — `max_sequence` counts abandoned rows, so the ID is never reissued and a restore into one cannot collide with a different record. Ordinary reads keep using `adr_allocation` and continue to hide abandoned rows.
3. A successful restore moves the allocation's `status` to `merged` inside the existing compare-and-set, because the repair has just written a readable body into the current worktree. The `WHERE` clause still pins the full pre-restore snapshot, so a concurrent change to any field — `status` included — still loses the race.

## Consequences

- The 18 [ORB-10479] narratives were restorable at their existing IDs, with no ID reallocated and no inline citation broken.
- Reviving the allocation keeps the invariant that a locally readable ADR has a live allocation row, so a later `resolve_adr_artifact` from another checkout reports `remote_artifact_unavailable` rather than `not_found`.
- The inactive-plus-CLI-subcommand pairing is now the established shape for operator-only tools; adding one without the subcommand ships an unreachable surface, which is what happened here.
- Cost: `restore_body_path_if_unchanged` now writes `status` as well as location, so it is no longer a pure relocation primitive. Any future caller that wants to move an allocation's body path *without* asserting the record is merged needs a separate function rather than reusing this one — the ADR-only `kind` guard is what keeps that blast radius small today.
- Cost: restore remains reachable only from a local CLI. Agent sessions and the MCP surface still cannot repair a lost body, so the repair depends on an operator noticing the loss; [F2026-07-163] stays open for the detection half of the problem.