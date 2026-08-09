## Context

Bubblewrap cannot bind-mount a path that does not exist, so a policy re-allow beneath the read-only `.orbit` mount only takes effect if its anchor is already on disk. Three separate mechanisms handled that, and all three were wrong in the same direction.

Materialization ran once, in `worktree_setup`, over a snapshot of the task's `context_files`, using the un-absolutized policy profile — a profile that omits the host-appended run roots the spawn actually enforces. The set could therefore neither match the kernel's view nor grow during the run.

Which targets were eligible was gated on a hardcoded `[(path, kind); 5]` table matched by exact tuple. The table duplicated the shipped policy's `denyModify` exception list, so the two could drift; and because membership included the filesystem kind, a grant naming a *file* beneath a directory in the table missed the match entirely. That case was reported roughly half an hour after the table shipped.

Finally, `compile_linux_bwrap_argv` returned nothing for a positive rule whose path was absent. The grant silently stayed under the read-only bind, and the agent received an unattributable `Read-only file system` mid-turn.

## Decision

Derive the write-grant set from the effective `ResolvedFsProfile` — every positive exact/subtree rule that is a narrow re-allow beneath an earlier deny — and materialize absent anchors in `spawn_linux_bwrap`, immediately before compiling argv from that same profile.

Anchor shape is derived from evidence in order: what is on disk, then the rule's syntax (`<root>/**` is necessarily a directory), then whether another rule nests beneath the anchor, then whether the leaf carries a file extension. Directory is the residual default, since a directory anchor still admits files created inside it while a file anchor admits nothing.

Creation is confined to the managed worktree: every component that root owns is symlink-checked, files use create-new semantics, and an existing anchor of the wrong type fails closed naming its path and rule. Anchors outside that root are host-owned and are reported rather than invented. `LinuxBwrapPlan` carries a `dropped_grants` list, and a grant left unmountable inside a managed worktree is a permanent spawn failure naming the path and the granting rule.

## Rejected alternatives

**Keep materialization in `worktree_setup` and widen the table.** The table is the defect, not its length: it duplicates the policy's exception list, and any table keyed by kind reproduces the file-under-granted-directory miss. Setup also cannot see the host-appended roots, so its grant set is structurally unable to match the enforced one.

**Re-derive grants continuously during the run.** A live child's mount namespace is fixed at `bwrap` exec; there is no way to add a mount to a running provider without re-spawning it. Per-spawn derivation is the finest granularity the backend actually supports, so the chosen point is not a compromise between fast and correct — it is the only place the question can be answered.

**Fail the run at setup when a task's context selector names a path policy denies.** Rejected: `context_files` are read references at least as often as write targets, so this fails runs that merely cite a policy or resources file as reading context. `linux_bwrap_write_grant_diagnostic` provides the attributable path-and-rule explanation without converting a read reference into a refusal.

## Consequences

- A granted path that does not exist yet is usable by the sandboxed process, and file-versus-directory no longer decides whether a grant takes effect.
- Adding a versioned `.orbit` path is now a one-line policy change; no Rust inventory tracks it.
- The grant set is recomputed per spawn from the enforced profile, so it cannot drift from the kernel's view.
- A denial is attributable before the provider starts, against a path and a rule, instead of as an EROFS inside an agent turn.
- Cost: an exact *file* grant that is absent, untracked, and un-ignored leaves an empty anchor in the worktree that `git add -A` would stage. It is empty and therefore visible in review rather than silent. In this repository `.orbit/config.toml` is tracked and `.orbit/config.yaml` is ignored, so no such anchor arises.
- Cost: anchor shape for an absent exact rule is inferred, not declared. Spelling a directory grant as `<path>/**` remains the way to state it unambiguously.
- Read-only git metadata for linked worktrees is unchanged and still out of scope.